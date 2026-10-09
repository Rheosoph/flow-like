// Native adaptation of RF-DETR, Copyright (c) 2025 Roboflow.
// Derived upstream work: Copyright (c) 2024 Baidu, 2021 Microsoft,
// Facebook, Inc. and its affiliates, and 2020 SenseTime.
// Licensed under Apache-2.0; see licenses/RF-DETR.
// Native adaptation of RF-DETR, Copyright (c) 2025 Roboflow.
// Licensed under Apache-2.0; see licenses/RF-DETR.
use crate::{
    BoundingBox, Detection, DetectionOptions, Error, Result, RfDetrConfig,
    detr_layers::{Attention, DeformableAttention, Mlp, linear, norm},
    detr_loss::{DetrClassificationLoss, DetrLossConfig, detr_loss},
    pretrained::SafeTensorReader,
    rfdetr_backbone::{BackboneDimensions, Projector, RfBackbone},
};
use burn::{
    module::{Module, Param},
    nn::{Initializer, LayerNorm, Linear},
    tensor::{
        Device, Tensor, TensorData,
        activation::{relu, sigmoid},
    },
};

#[derive(Clone, Debug)]
struct Dimensions {
    backbone: BackboneDimensions,
    hidden: usize,
    self_heads: usize,
    cross_heads: usize,
    points: usize,
    feedforward: usize,
    layers: usize,
    queries: usize,
    groups: usize,
}
impl Dimensions {
    fn standard(config: &RfDetrConfig) -> Self {
        let variant = config.variant;
        Self {
            backbone: BackboneDimensions {
                embedding: 384,
                depth: 12,
                heads: 6,
                patch: variant.patch_size(),
                grid: variant.positional_grid(),
                windows: variant.windows(),
                stages: variant.feature_stages().to_vec(),
            },
            hidden: 256,
            self_heads: 8,
            cross_heads: 16,
            points: 2,
            feedforward: 2048,
            layers: variant.decoder_layers(),
            queries: RfDetrConfig::QUERIES,
            groups: RfDetrConfig::TRAINING_GROUPS,
        }
    }
}

#[derive(Module, Debug)]
struct Decoder {
    attention: Attention,
    cross: DeformableAttention,
    first: Linear,
    second: Linear,
    norm1: LayerNorm,
    norm2: LayerNorm,
    norm3: LayerNorm,
}
impl Decoder {
    fn new(d: &Dimensions, device: &Device) -> Self {
        Self {
            attention: Attention::new(d.hidden, d.self_heads, device),
            cross: DeformableAttention::new(d.hidden, d.cross_heads, vec![d.points], device),
            first: linear(d.hidden, d.feedforward, device),
            second: linear(d.feedforward, d.hidden, device),
            norm1: norm(d.hidden, device),
            norm2: norm(d.hidden, device),
            norm3: norm(d.hidden, device),
        }
    }
    fn forward(
        &self,
        target: Tensor<3>,
        position: Tensor<3>,
        references: Tensor<3>,
        memory: Tensor<3>,
        grid: [usize; 2],
        groups: usize,
    ) -> Tensor<3> {
        let [batch, queries, hidden] = target.dims();
        let positioned =
            (target.clone() + position.clone()).reshape([batch * groups, queries / groups, hidden]);
        let attended = self
            .attention
            .forward(
                positioned.clone(),
                positioned,
                target
                    .clone()
                    .reshape([batch * groups, queries / groups, hidden]),
            )
            .reshape([batch, queries, hidden]);
        let target = self.norm1.forward(target + attended);
        let attended = self
            .cross
            .forward(target.clone() + position, references, memory, &[grid]);
        let target = self.norm2.forward(target + attended);
        self.norm3
            .forward(target.clone() + self.second.forward(relu(self.first.forward(target))))
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
        self.cross = self.cross.import_without_point_scale(
            &format!("{prefix}.cross_attn"),
            reader,
            device,
        )?;
        self.first = reader.linear(self.first, &format!("{prefix}.linear1"), device)?;
        self.second = reader.linear(self.second, &format!("{prefix}.linear2"), device)?;
        self.norm1 = reader.layer_norm(self.norm1, &format!("{prefix}.norm1"), device)?;
        self.norm2 = reader.layer_norm(self.norm2, &format!("{prefix}.norm2"), device)?;
        self.norm3 = reader.layer_norm(self.norm3, &format!("{prefix}.norm3"), device)?;
        Ok(self)
    }
}

#[derive(Module, Debug)]
struct EncoderHead {
    projection: Linear,
    norm: LayerNorm,
    classes: Linear,
    boxes: Mlp,
}
impl EncoderHead {
    fn new(hidden: usize, classes: usize, device: &Device) -> Self {
        Self {
            projection: linear(hidden, hidden, device),
            norm: norm(hidden, device),
            classes: classifier(hidden, classes, device),
            boxes: Mlp::new(hidden, hidden, 4, 3, device).zero_last(),
        }
    }
}

/// RF-DETR detection with the released windowed ViT-S and P4 projector geometry.
/// Inputs are RGB NCHW with ImageNet normalization. The decoder retains all 13
/// independent query groups during training and uses the first group for inference.
#[derive(Module, Debug)]
pub struct RfDetr {
    backbone: RfBackbone,
    projector: Projector,
    decoder: Vec<Decoder>,
    decoder_norm: LayerNorm,
    reference_head: Mlp,
    encoder_heads: Vec<EncoderHead>,
    query_features: Param<Tensor<2>>,
    query_references: Param<Tensor<2>>,
    classes: Linear,
    boxes: Mlp,
    #[module(skip)]
    dimensions: Dimensions,
}
impl RfDetr {
    pub fn new(config: &RfDetrConfig, device: &Device) -> Result<Self> {
        config.validate()?;
        Ok(Self::with_dimensions(
            config.classes,
            Dimensions::standard(config),
            device,
        ))
    }
    fn with_dimensions(classes: usize, d: Dimensions, device: &Device) -> Self {
        Self {
            backbone: RfBackbone::new(d.backbone.clone(), device),
            projector: Projector::new(
                d.backbone.embedding * d.backbone.stages.len(),
                d.hidden,
                device,
            ),
            decoder: (0..d.layers).map(|_| Decoder::new(&d, device)).collect(),
            decoder_norm: norm(d.hidden, device),
            reference_head: Mlp::new(2 * d.hidden, d.hidden, d.hidden, 2, device),
            encoder_heads: (0..d.groups)
                .map(|_| EncoderHead::new(d.hidden, classes, device))
                .collect(),
            query_features: Initializer::Normal {
                mean: 0.0,
                std: 1.0,
            }
            .init([d.queries * d.groups, d.hidden], device),
            query_references: Initializer::Zeros.init([d.queries * d.groups, 4], device),
            classes: classifier(d.hidden, classes, device),
            boxes: Mlp::new(d.hidden, d.hidden, 4, 3, device).zero_last(),
            dimensions: d,
        }
    }
    pub fn forward(&self, input: Tensor<4>, training: bool) -> Result<RfDetrOutput> {
        let [batch, channels, height, width] = input.dims();
        let d = &self.dimensions;
        let divisor = d.backbone.patch * d.backbone.windows;
        let patches = (height / d.backbone.patch)
            .checked_mul(width / d.backbone.patch)
            .unwrap_or(0);
        if batch == 0
            || channels != 3
            || !height.is_multiple_of(divisor)
            || !width.is_multiple_of(divisor)
            || !(d.queries..=4096).contains(&patches)
        {
            return Err(Error::Invalid(format!(
                "RF-DETR requires RGB dimensions divisible by {divisor}, with {} to 4096 patches",
                d.queries
            )));
        }
        let groups = if training { d.groups } else { 1 };
        let projected = self.projector.forward(self.backbone.forward(input));
        let [_, hidden, rows, columns] = projected.dims();
        let memory = projected
            .reshape([batch, hidden, rows * columns])
            .swap_dims(1, 2);
        let device = memory.device();
        let (proposals, valid) = proposals([rows, columns], &device);
        let selected_memory = memory.clone() * valid;
        let proposals = proposals.expand([batch, rows * columns, 4]);
        let mut encoder_logits = Vec::with_capacity(groups);
        let mut encoder_boxes = Vec::with_capacity(groups);
        for head in &self.encoder_heads[..groups] {
            let features = head
                .norm
                .forward(head.projection.forward(selected_memory.clone()));
            let logits = head.classes.forward(features.clone());
            let indices = logits
                .clone()
                .max_dim(2)
                .reshape([batch, rows * columns])
                .topk_with_indices(d.queries, 1)
                .1;
            let features = features.gather(
                1,
                indices
                    .clone()
                    .reshape([batch, d.queries, 1])
                    .expand([batch, d.queries, hidden]),
            );
            let priors = proposals.clone().gather(
                1,
                indices
                    .reshape([batch, d.queries, 1])
                    .expand([batch, d.queries, 4]),
            );
            encoder_logits.push(head.classes.forward(features.clone()));
            encoder_boxes.push(refine_boxes(head.boxes.forward(features), priors));
        }
        let encoder = RfDetrPrediction {
            logits: Tensor::cat(encoder_logits, 1),
            boxes: Tensor::cat(encoder_boxes, 1),
        };
        let queries = d.queries * groups;
        let learned_refs = self
            .query_references
            .val()
            .slice([0..queries, 0..4])
            .reshape([1, queries, 4])
            .expand([batch, queries, 4]);
        let references = refine_boxes(learned_refs, encoder.boxes.clone().detach());
        let position = self
            .reference_head
            .forward(sine_box_embedding(references.clone(), hidden / 2));
        let mut target = self
            .query_features
            .val()
            .slice([0..queries, 0..hidden])
            .reshape([1, queries, hidden])
            .expand([batch, queries, hidden]);
        let mut outputs = Vec::with_capacity(self.decoder.len());
        for layer in &self.decoder {
            target = layer.forward(
                target,
                position.clone(),
                references.clone(),
                memory.clone(),
                [rows, columns],
                groups,
            );
            let normalized = self.decoder_norm.forward(target.clone());
            outputs.push(RfDetrPrediction {
                logits: self.classes.forward(normalized.clone()),
                boxes: refine_boxes(self.boxes.forward(normalized), references.clone()),
            });
        }
        let final_output = outputs.pop().expect("RF-DETR has decoder layers");
        Ok(RfDetrOutput {
            logits: final_output.logits,
            boxes: final_output.boxes,
            auxiliary: outputs,
            encoder,
            groups,
        })
    }
    pub fn reset_classifier(mut self, classes: usize, device: &Device) -> Self {
        assert!(
            (1..=1024).contains(&classes),
            "Invalid RF-DETR score channel count"
        );
        self.classes = classifier(self.dimensions.hidden, classes, device);
        for head in &mut self.encoder_heads {
            head.classes = classifier(self.dimensions.hidden, classes, device);
        }
        self
    }
    pub fn configure_fine_tuning(self, freeze_backbone: bool) -> Self {
        let mut model = self.unfreeze();
        if freeze_backbone {
            model.backbone = model.backbone.freeze();
            model.projector = model.projector.freeze();
        }
        model
    }
    pub(crate) fn import_safetensors(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.backbone = self
            .backbone
            .import("backbone.0.encoder.encoder", reader, device)?;
        self.projector = self
            .projector
            .import("backbone.0.projector", reader, device)?;
        self.decoder = self
            .decoder
            .into_iter()
            .enumerate()
            .map(|(index, layer)| {
                layer.import(
                    &format!("transformer.decoder.layers.{index}"),
                    reader,
                    device,
                )
            })
            .collect::<Result<_>>()?;
        self.decoder_norm =
            reader.layer_norm(self.decoder_norm, "transformer.decoder.norm", device)?;
        self.reference_head =
            self.reference_head
                .import("transformer.decoder.ref_point_head", reader, device)?;
        for (index, head) in self.encoder_heads.iter_mut().enumerate() {
            head.projection = reader.linear(
                head.projection.clone(),
                &format!("transformer.enc_output.{index}"),
                device,
            )?;
            head.norm = reader.layer_norm(
                head.norm.clone(),
                &format!("transformer.enc_output_norm.{index}"),
                device,
            )?;
            head.classes = reader.linear(
                head.classes.clone(),
                &format!("transformer.enc_out_class_embed.{index}"),
                device,
            )?;
            head.boxes = head.boxes.clone().import(
                &format!("transformer.enc_out_bbox_embed.{index}"),
                reader,
                device,
            )?;
        }
        let d = &self.dimensions;
        self.query_features = Param::from_tensor(reader.tensor(
            "query_feat.weight",
            [d.queries * d.groups, d.hidden],
            device,
        )?);
        self.query_references = Param::from_tensor(reader.tensor(
            "refpoint_embed.weight",
            [d.queries * d.groups, 4],
            device,
        )?);
        self.classes = reader.linear(self.classes, "class_embed", device)?;
        self.boxes = self.boxes.import("bbox_embed", reader, device)?;
        Ok(self)
    }
}

#[derive(Clone, Debug)]
pub struct RfDetrPrediction {
    pub logits: Tensor<3>,
    pub boxes: Tensor<3>,
}
#[derive(Clone, Debug)]
pub struct RfDetrOutput {
    pub logits: Tensor<3>,
    pub boxes: Tensor<3>,
    pub auxiliary: Vec<RfDetrPrediction>,
    pub encoder: RfDetrPrediction,
    pub groups: usize,
}
impl RfDetrOutput {
    pub fn loss(&self, targets: &[Vec<BoundingBox>]) -> Result<Tensor<1>> {
        let config = DetrLossConfig {
            classification: DetrClassificationLoss::IouAwareBce,
            ..Default::default()
        };
        let mut loss = self.logits.clone().sum() * 0.0;
        for (logits, boxes) in std::iter::once((&self.logits, &self.boxes))
            .chain(self.auxiliary.iter().map(|out| (&out.logits, &out.boxes)))
            .chain(std::iter::once((&self.encoder.logits, &self.encoder.boxes)))
        {
            let [batch, queries, classes] = logits.dims();
            if self.groups == 0 || !queries.is_multiple_of(self.groups) {
                return Err(Error::Invalid(
                    "RF-DETR query groups do not divide prediction count".into(),
                ));
            }
            let per_group = queries / self.groups;
            for group in 0..self.groups {
                let range = group * per_group..(group + 1) * per_group;
                loss = loss
                    + detr_loss(
                        logits.clone().slice([0..batch, range.clone(), 0..classes]),
                        boxes.clone().slice([0..batch, range, 0..4]),
                        targets,
                        &config,
                    )? / self.groups as f64;
            }
        }
        Ok(loss)
    }
    /// RF-DETR ranks sigmoid scores across query/class pairs without applying NMS.
    pub fn detections(&self, options: &DetectionOptions) -> Result<Vec<Vec<Detection>>> {
        options.validate()?;
        let [batch, queries, classes] = self.logits.dims();
        if self.boxes.dims() != [batch, queries, 4] || self.groups != 1 {
            return Err(Error::Invalid(
                "Detection postprocessing requires one inference query group".into(),
            ));
        }
        let scores = sigmoid(self.logits.clone())
            .into_data()
            .try_to_vec::<f32>()
            .map_err(|e| Error::Invalid(e.to_string()))?;
        let boxes = self
            .boxes
            .clone()
            .into_data()
            .try_to_vec::<f32>()
            .map_err(|e| Error::Invalid(e.to_string()))?;
        if scores.iter().chain(&boxes).any(|value| !value.is_finite()) {
            return Err(Error::Invalid(
                "RF-DETR produced non-finite scores or boxes".into(),
            ));
        }
        Ok((0..batch)
            .map(|image| {
                let mut candidates = Vec::new();
                for query in 0..queries {
                    let coords =
                        &boxes[(image * queries + query) * 4..(image * queries + query + 1) * 4];
                    let [cx, cy, width, height] = [coords[0], coords[1], coords[2], coords[3]];
                    let bounds = [
                        (cx - width / 2.0).clamp(0.0, 1.0),
                        (cy - height / 2.0).clamp(0.0, 1.0),
                        (cx + width / 2.0).clamp(0.0, 1.0),
                        (cy + height / 2.0).clamp(0.0, 1.0),
                    ];
                    if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
                        continue;
                    }
                    for class_id in 0..classes {
                        let confidence = scores[(image * queries + query) * classes + class_id];
                        if confidence >= options.confidence_threshold {
                            candidates.push(Detection {
                                class_id,
                                confidence,
                                x_min: bounds[0],
                                y_min: bounds[1],
                                x_max: bounds[2],
                                y_max: bounds[3],
                            });
                        }
                    }
                }
                candidates.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
                candidates.truncate(options.max_detections.min(300));
                candidates
            })
            .collect())
    }
}

fn classifier(input: usize, outputs: usize, device: &Device) -> Linear {
    let mut layer = linear(input, outputs, device);
    layer.bias = Some(
        Initializer::Constant {
            value: -(99.0_f64).ln(),
        }
        .init([outputs], device),
    );
    layer
}
fn refine_boxes(delta: Tensor<3>, reference: Tensor<3>) -> Tensor<3> {
    let [batch, count, _] = delta.dims();
    let xy = reference.clone().slice([0..batch, 0..count, 0..2]);
    let wh = reference.slice([0..batch, 0..count, 2..4]);
    let center = delta.clone().slice([0..batch, 0..count, 0..2]) * wh.clone() + xy;
    let size = delta.slice([0..batch, 0..count, 2..4]).exp() * wh;
    Tensor::cat(vec![center, size], 2)
}
fn proposals([height, width]: [usize; 2], device: &Device) -> (Tensor<3>, Tensor<3>) {
    let mut boxes = Vec::with_capacity(height * width * 4);
    let mut valid = Vec::<f32>::with_capacity(height * width);
    for y in 0..height {
        for x in 0..width {
            let cx = (x as f32 + 0.5) / width as f32;
            let cy = (y as f32 + 0.5) / height as f32;
            let keep = cx > 0.01 && cx < 0.99 && cy > 0.01 && cy < 0.99;
            boxes.extend(if keep { [cx, cy, 0.05, 0.05] } else { [0.0; 4] });
            valid.push(if keep { 1.0 } else { 0.0 });
        }
    }
    (
        Tensor::from_data(TensorData::new(boxes, [1, height * width, 4]), device),
        Tensor::from_data(TensorData::new(valid, [1, height * width, 1]), device),
    )
}
fn sine_box_embedding(boxes: Tensor<3>, dim: usize) -> Tensor<3> {
    let [batch, count, _] = boxes.dims();
    let frequencies = Tensor::<3>::from_data(
        TensorData::new(
            (0..dim / 2)
                .map(|i| 10000.0_f32.powf((2 * i) as f32 / dim as f32))
                .collect::<Vec<_>>(),
            [1, 1, dim / 2],
        ),
        &boxes.device(),
    );
    let mut embedding = Vec::with_capacity(4);
    for coordinate in [1, 0, 2, 3] {
        let phase = boxes
            .clone()
            .slice([0..batch, 0..count, coordinate..coordinate + 1])
            * std::f64::consts::TAU
            / frequencies.clone();
        let pairs = Tensor::stack::<4>(vec![phase.clone().sin(), phase.cos()], 3);
        embedding.push(pairs.reshape([batch, count, dim]));
    }
    Tensor::cat(embedding, 2)
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;

    fn small() -> Dimensions {
        Dimensions {
            backbone: BackboneDimensions {
                embedding: 12,
                depth: 4,
                heads: 3,
                patch: 2,
                grid: 4,
                windows: 2,
                stages: vec![1, 2, 3, 4],
            },
            hidden: 16,
            self_heads: 2,
            cross_heads: 4,
            points: 2,
            feedforward: 24,
            layers: 2,
            queries: 4,
            groups: 2,
        }
    }
    fn input(device: &Device) -> Tensor<4> {
        Tensor::from_data(
            TensorData::new(
                (0..2 * 3 * 8 * 12)
                    .map(|i| (i as f32 * 0.07).sin())
                    .collect::<Vec<_>>(),
                [2, 3, 8, 12],
            ),
            device,
        )
    }
    fn data<const D: usize>(tensor: Tensor<D>) -> Vec<f32> {
        tensor.into_data().try_to_vec().unwrap()
    }
    fn close<const D: usize>(actual: Tensor<D>, expected: Tensor<D>, tolerance: f32) {
        assert_eq!(actual.dims(), expected.dims());
        for (a, b) in data(actual).into_iter().zip(data(expected)) {
            assert!((a - b).abs() < tolerance, "{a} != {b}");
        }
    }
    fn targets() -> Vec<Vec<BoundingBox>> {
        vec![
            vec![BoundingBox {
                class_id: 0,
                x_min: 0.1,
                y_min: 0.2,
                x_max: 0.6,
                y_max: 0.7,
            }],
            vec![BoundingBox {
                class_id: 2,
                x_min: 0.45,
                y_min: 0.15,
                x_max: 0.9,
                y_max: 0.5,
            }],
        ]
    }

    #[test]
    fn grouped_training_preserves_the_inference_group_and_backpropagates() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        let model = RfDetr::with_dimensions(3, small(), &device).train();
        let inference = model.forward(input(&device), false).unwrap();
        let training = model.forward(input(&device), true).unwrap();
        assert_eq!(inference.logits.dims(), [2, 4, 3]);
        assert_eq!(training.logits.dims(), [2, 8, 3]);
        close(
            training.logits.clone().slice([0..2, 0..4, 0..3]),
            inference.logits,
            1e-5,
        );
        close(
            training.boxes.clone().slice([0..2, 0..4, 0..4]),
            inference.boxes,
            1e-5,
        );
        assert_eq!(training.auxiliary.len(), 1);
        let loss = training.loss(&targets()).unwrap();
        assert!(data(loss.clone())[0].is_finite());
        let grads = loss.backward();
        let queries = data(
            model
                .query_features
                .val()
                .grad(&grads)
                .expect("query features gradient"),
        );
        for group in queries.chunks(4 * 16) {
            assert!(
                group.iter().any(|v| v.abs() > 1e-8),
                "Each query group must learn"
            );
        }
        for head in &model.encoder_heads {
            let grad = data(
                head.classes
                    .weight
                    .val()
                    .grad(&grads)
                    .expect("encoder classification gradient"),
            );
            assert!(grad.iter().any(|v| v.abs() > 1e-8));
        }
        assert!(
            model
                .boxes
                .layers
                .last()
                .unwrap()
                .weight
                .val()
                .grad(&grads)
                .is_some()
        );
    }

    #[test]
    fn records_restore_predictions_and_head_replacement_supports_fine_tuning() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        let model = RfDetr::with_dimensions(3, small(), &device).train();
        let before = model.forward(input(&device), false).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rf.bpk");
        model.save_file(&path).unwrap();
        let model = RfDetr::with_dimensions(3, small(), &device)
            .try_load_file(path)
            .unwrap();
        let after = model.forward(input(&device), false).unwrap();
        close(before.logits, after.logits, 1e-6);
        close(before.boxes, after.boxes, 1e-6);
        let model = model
            .reset_classifier(5, &device)
            .configure_fine_tuning(true)
            .train();
        let output = model.forward(input(&device), true).unwrap();
        assert_eq!(output.logits.dims(), [2, 8, 5]);
        let gradients = output.loss(&targets()).unwrap().backward();
        assert!(model.classes.weight.val().grad(&gradients).is_some());
        let mut visitor = GradientVisitor {
            gradients: &gradients,
            present: false,
        };
        model.backbone.visit(&mut visitor);
        model.projector.visit(&mut visitor);
        assert!(!visitor.present, "Frozen backbone cannot receive gradients");
    }
    struct GradientVisitor<'a> {
        gradients: &'a burn::tensor::Gradients,
        present: bool,
    }
    impl burn::module::ModuleVisitor for GradientVisitor<'_> {
        fn visit_float<const D: usize>(&mut self, parameter: &Param<Tensor<D>>) {
            self.present |= parameter.val().grad(self.gradients).is_some();
        }
    }

    #[test]
    fn postprocessing_preserves_overlapping_queries_without_nms() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, false).unwrap();
        let logits = Tensor::from_data([[[4.0f32, -5.0], [3.0, 2.0]]], &device);
        let boxes = Tensor::from_data([[[0.5f32, 0.5, 0.4, 0.4], [0.5, 0.5, 0.4, 0.4]]], &device);
        let output = RfDetrOutput {
            encoder: RfDetrPrediction {
                logits: logits.clone(),
                boxes: boxes.clone(),
            },
            logits,
            boxes,
            auxiliary: Vec::new(),
            groups: 1,
        };
        let detections = output
            .detections(&DetectionOptions {
                confidence_threshold: 0.5,
                iou_threshold: 0.0,
                max_detections: 3,
            })
            .unwrap();
        assert_eq!(detections[0].len(), 3);
        assert_eq!(
            detections[0].iter().map(|d| d.class_id).collect::<Vec<_>>(),
            [0, 0, 1]
        );
        assert!(detections[0][0].confidence > detections[0][1].confidence);
    }

    #[test]
    fn official_variants_have_distinct_checkpoint_geometry() {
        for variant in [
            crate::RfDetrVariant::Base,
            crate::RfDetrVariant::Nano,
            crate::RfDetrVariant::Small,
            crate::RfDetrVariant::Medium,
            crate::RfDetrVariant::Large2026,
        ] {
            let config = RfDetrConfig {
                variant,
                classes: 91,
            };
            let resolution = variant.resolution();
            config
                .validate_input_shape(&[1, 3, resolution, resolution])
                .unwrap();
            assert!(
                config
                    .validate_input_shape(&[1, 3, resolution + 1, resolution])
                    .is_err()
            );
        }
        assert_eq!(crate::RfDetrVariant::Base.feature_stages(), [2, 5, 8, 11]);
        assert_eq!(crate::RfDetrVariant::Nano.feature_stages(), [3, 6, 9, 12]);
    }

    #[test]
    #[ignore = "requires the official RF-DETR Nano PyTorch ZIP checkpoint"]
    fn public_nano_pytorch_checkpoint_imports_and_runs_natively() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let path = std::env::var("FLOW_LIKE_RFDETR_NANO_WEIGHTS").expect("Set FLOW_LIKE_RFDETR_NANO_WEIGHTS to the official nano_coco/checkpoint_best_regular.pth");
        assert_eq!(
            crate::pretrained::file_sha256(std::path::Path::new(&path)).unwrap(),
            "d8d6b9ee57d4d0ed2b1f305163624712a0532cb7bce0c747317984fc5457440d"
        );
        let checkpoint = std::fs::read(path).unwrap();
        let (bytes, format) =
            crate::pytorch::convert_pytorch_weights(&checkpoint, 512 * 1024 * 1024).unwrap();
        assert_eq!(format, "pytorch_zip:model");
        let mut reader = SafeTensorReader::new(&bytes).unwrap();
        let device = crate::backend::device(&crate::BackendChoice::Cpu, false).unwrap();
        let config = RfDetrConfig {
            variant: crate::RfDetrVariant::Nano,
            classes: 91,
        };
        let model = RfDetr::new(&config, &device)
            .unwrap()
            .import_safetensors(&mut reader, &device)
            .unwrap();
        reader.finish().unwrap();
        let input = Tensor::from_data(
            TensorData::new(
                (0..3 * 384 * 384)
                    .map(|index| (index % 251) as f32 / 250.0)
                    .collect::<Vec<_>>(),
                [1, 3, 384, 384],
            ),
            &device,
        );
        let output = model.forward(input, false).unwrap();
        assert_eq!(output.logits.dims(), [1, 300, 91]);
        assert_eq!(output.boxes.dims(), [1, 300, 4]);
        assert!(
            data(output.logits.clone())
                .iter()
                .all(|value| value.is_finite())
        );
        assert!(
            data(output.boxes.clone())
                .iter()
                .all(|value| value.is_finite())
        );
        // Reference: RF-DETR b6814eaba40e30d779562f999e909d785e146738, PyTorch 2.14.
        // Evaluate the published f32 weights and this f32 input in f64. The projector
        // amplifies ordinary f32 rounding: upstream f32 differs from that oracle by
        // up to 1.32e-3 in logits. Keep the native f32 output within 1e-3 of this
        // higher-precision oracle.
        if let Ok(path) = std::env::var("FLOW_LIKE_RFDETR_NANO_REFERENCE") {
            let reference: serde_json::Value =
                serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            for (name, actual) in [
                ("logits", data(output.logits.clone())),
                ("boxes", data(output.boxes.clone())),
            ] {
                let expected = reference[name].as_array().unwrap();
                assert_eq!(actual.len(), expected.len());
                let maximum = actual
                    .into_iter()
                    .zip(expected)
                    .map(|(actual, expected)| (actual as f64 - expected.as_f64().unwrap()).abs())
                    .fold(0.0f64, f64::max);
                assert!(maximum < 1e-3, "{name} maximum upstream error: {maximum}");
                eprintln!("RF-DETR Nano {name} maximum upstream error: {maximum}");
            }
        }
        let detections = output
            .detections(&DetectionOptions {
                confidence_threshold: 0.0,
                max_detections: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(detections[0].len(), 10);
    }
}

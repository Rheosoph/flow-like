use crate::mask_rcnn_types::{
    InstanceDetection, InstanceTarget, MaskRcnnConfig, MaskRcnnPrediction,
};
use crate::{Detection, DetectionOptions, Error, Result, TensorData};
use burn::{
    module::Module,
    nn::{
        Linear, LinearConfig, PaddingConfig2d,
        conv::{Conv2d, Conv2dConfig, ConvTranspose2d, ConvTranspose2dConfig},
        loss::{CrossEntropyLossConfig, Reduction, SmoothL1LossConfig},
    },
    tensor::{
        Device, Distribution, Int, Tensor, TensorData as BurnData,
        activation::{relu, softmax},
    },
};

type BoxCoords = [f32; 4];
#[derive(Clone, Debug)]
pub struct Roi {
    pub image: usize,
    pub bounds: BoxCoords,
}

#[derive(Module, Debug)]
struct Residual {
    first: Conv2d,
    second: Conv2d,
    projection: Option<Conv2d>,
}
impl Residual {
    fn new(input: usize, output: usize, stride: usize, device: &Device) -> Self {
        let conv = |a, b, k, s| {
            Conv2dConfig::new([a, b], [k, k])
                .with_stride([s, s])
                .with_padding(PaddingConfig2d::Explicit(k / 2, k / 2, k / 2, k / 2))
                .init(device)
        };
        Self {
            first: conv(input, output, 3, stride),
            second: conv(output, output, 3, 1),
            projection: (stride != 1 || input != output).then(|| conv(input, output, 1, stride)),
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let skip = self
            .projection
            .as_ref()
            .map_or_else(|| input.clone(), |layer| layer.forward(input.clone()));
        relu(self.second.forward(relu(self.first.forward(input))) + skip)
    }
}

/// Two-stage instance detector with a compact residual backbone, RPN,
/// differentiable ROIAlign, class-specific box regression and FCN mask heads.
#[derive(Module, Debug)]
pub struct MaskRcnn {
    backbone: Vec<Residual>,
    rpn: Conv2d,
    objectness: Conv2d,
    rpn_boxes: Conv2d,
    fc1: Linear,
    fc2: Linear,
    classes: Linear,
    boxes: Linear,
    mask_convs: Vec<Conv2d>,
    mask_up: ConvTranspose2d,
    mask_logits: Conv2d,
    #[module(skip)]
    config: MaskRcnnConfig,
}
impl MaskRcnn {
    pub fn new(config: &MaskRcnnConfig, device: &Device) -> Result<Self> {
        config.validate()?;
        let c = config.base_channels;
        let a = config.anchor_scales.len() * config.anchor_ratios.len();
        let hidden = c * 4;
        let same = |input, output| {
            Conv2dConfig::new([input, output], [3, 3])
                .with_padding(PaddingConfig2d::Same)
                .init(device)
        };
        Ok(Self {
            backbone: vec![
                Residual::new(config.input_channels, c, 2, device),
                Residual::new(c, c, 1, device),
                Residual::new(c, c * 2, 2, device),
                Residual::new(c * 2, c * 2, 1, device),
            ],
            rpn: same(c * 2, c * 2),
            objectness: Conv2dConfig::new([c * 2, a], [1, 1]).init(device),
            rpn_boxes: Conv2dConfig::new([c * 2, a * 4], [1, 1]).init(device),
            fc1: LinearConfig::new(c * 2 * config.roi_size * config.roi_size, hidden).init(device),
            fc2: LinearConfig::new(hidden, hidden).init(device),
            classes: LinearConfig::new(hidden, config.classes + 1).init(device),
            boxes: LinearConfig::new(hidden, config.classes * 4).init(device),
            mask_convs: vec![
                same(c * 2, c * 2),
                same(c * 2, c * 2),
                same(c * 2, c * 2),
                same(c * 2, c * 2),
            ],
            mask_up: ConvTranspose2dConfig::new([c * 2, c * 2], [2, 2])
                .with_stride([2, 2])
                .init(device),
            mask_logits: Conv2dConfig::new([c * 2, config.classes], [1, 1]).init(device),
            config: config.clone(),
        })
    }
    pub fn forward(
        &self,
        input: Tensor<4>,
        targets: Option<&[Vec<InstanceTarget>]>,
    ) -> Result<MaskRcnnOutput> {
        let [batch, channels, height, width] = input.dims();
        if batch == 0
            || batch > 32
            || channels != self.config.input_channels
            || height < 4
            || width < 4
            || height.checked_mul(width).is_none_or(|v| v > 4_194_304)
        {
            return Err(Error::Invalid(
                "Invalid Mask R-CNN image batch dimensions".into(),
            ));
        }
        if let Some(targets) = targets {
            validate_targets(targets, batch, self.config.classes, height, width)?;
        }
        let sampling_seed = targets.map(|_| {
            values(Tensor::<1>::random(
                [1],
                Distribution::Uniform(0.0, 1.0),
                &input.device(),
            ))[0]
                .to_bits() as u64
        });
        let mut features = input;
        for block in &self.backbone {
            features = block.forward(features);
        }
        let [_, _, fh, fw] = features.dims();
        let anchor_count = fh.checked_mul(fw).and_then(|n| {
            n.checked_mul(self.config.anchor_scales.len() * self.config.anchor_ratios.len())
        });
        if anchor_count.is_none_or(|n| n > 1_000_000) {
            return Err(Error::Invalid("Mask R-CNN anchor budget exceeded".into()));
        }
        let max_rois = batch
            * if targets.is_some() {
                self.config.training_samples
            } else {
                self.config.proposals
            };
        let mask_elements = max_rois
            .checked_mul(self.config.base_channels * 2)
            .and_then(|n| n.checked_mul(self.config.mask_size * self.config.mask_size));
        if mask_elements.is_none_or(|n| n > 64 * 1024 * 1024) {
            return Err(Error::Invalid(
                "Mask R-CNN proposal feature budget exceeded".into(),
            ));
        }
        let anchors = anchors(fh, fw, height, width, &self.config);
        if anchors.len() > 1_000_000 {
            return Err(Error::Invalid("Mask R-CNN anchor budget exceeded".into()));
        }
        let rpn = relu(self.rpn.forward(features.clone()));
        let objectness = self
            .objectness
            .forward(rpn.clone())
            .permute([0, 2, 3, 1])
            .reshape([batch, anchors.len()]);
        let deltas =
            self.rpn_boxes
                .forward(rpn)
                .permute([0, 2, 3, 1])
                .reshape([batch, anchors.len(), 4]);
        let scores = values(objectness.clone());
        let offsets = values(deltas.clone());
        let mut rois = Vec::new();
        for image in 0..batch {
            let mut proposals = proposals(
                &anchors,
                &scores[image * anchors.len()..(image + 1) * anchors.len()],
                &offsets[image * anchors.len() * 4..(image + 1) * anchors.len() * 4],
                self.config.proposals,
            )?;
            if let Some(targets) = targets {
                proposals.extend(targets[image].iter().map(|target| coords(&target.bbox)));
                proposals = sample_rois(
                    proposals,
                    &targets[image],
                    self.config.training_samples,
                    sampling_seed.unwrap().wrapping_add(image as u64),
                );
            }
            if proposals.is_empty() {
                proposals.push([0.0, 0.0, 1.0, 1.0]);
            }
            rois.extend(proposals.into_iter().map(|bounds| Roi { image, bounds }));
        }
        let pooled = roi_align(features.clone(), &rois, self.config.roi_size, 2)?;
        let hidden = relu(
            self.fc2
                .forward(relu(self.fc1.forward(pooled.flatten(1, 3)))),
        );
        let class_logits = self.classes.forward(hidden.clone());
        let box_deltas = self.boxes.forward(hidden);
        let mut mask_valid = vec![true; rois.len()];
        let mask_rois = if targets.is_none() {
            let scores = values(class_logits.clone());
            let offsets = values(box_deltas.clone());
            if scores.iter().chain(&offsets).any(|v| !v.is_finite()) {
                return Err(Error::Invalid(
                    "ROI heads produced non-finite values".into(),
                ));
            }
            rois.iter()
                .enumerate()
                .map(|(i, roi)| {
                    let class = (1..=self.config.classes)
                        .max_by(|a, b| {
                            scores[i * (self.config.classes + 1) + a]
                                .total_cmp(&scores[i * (self.config.classes + 1) + b])
                        })
                        .unwrap_or(1)
                        - 1;
                    let at = (i * self.config.classes + class) * 4;
                    let decoded = decode_box(roi.bounds, offsets[at..at + 4].try_into().unwrap());
                    mask_valid[i] = valid_box(decoded);
                    Roi {
                        image: roi.image,
                        bounds: if mask_valid[i] { decoded } else { roi.bounds },
                    }
                })
                .collect()
        } else {
            rois.clone()
        };
        let mut mask = roi_align(features, &mask_rois, self.config.mask_size / 2, 2)?;
        for conv in &self.mask_convs {
            mask = relu(conv.forward(mask));
        }
        let mask_logits = self.mask_logits.forward(relu(self.mask_up.forward(mask)));
        Ok(MaskRcnnOutput {
            objectness,
            deltas,
            anchors,
            rois,
            mask_rois,
            mask_valid,
            class_logits,
            box_deltas,
            mask_logits,
            batch,
            height,
            width,
            classes: self.config.classes,
        })
    }
}

pub struct MaskRcnnOutput {
    objectness: Tensor<2>,
    deltas: Tensor<3>,
    anchors: Vec<BoxCoords>,
    rois: Vec<Roi>,
    mask_rois: Vec<Roi>,
    mask_valid: Vec<bool>,
    class_logits: Tensor<2>,
    box_deltas: Tensor<2>,
    mask_logits: Tensor<4>,
    batch: usize,
    height: usize,
    width: usize,
    classes: usize,
}
impl MaskRcnnOutput {
    pub fn loss(self, targets: &[Vec<InstanceTarget>]) -> Result<Tensor<1>> {
        validate_targets(targets, self.batch, self.classes, self.height, self.width)?;
        let device = self.objectness.device();
        let count = self.anchors.len();
        let raw = values(self.objectness.clone());
        let mut selected = Vec::new();
        let mut obj_targets = Vec::new();
        let mut positive = Vec::new();
        let mut box_targets = Vec::new();
        for (image, truth) in targets.iter().enumerate() {
            let assignment = assign_anchors(&self.anchors, truth);
            let mut pos = (0..count)
                .filter(|i| assignment[*i].is_some_and(|v| v.1 >= 0.7 || v.2))
                .collect::<Vec<_>>();
            let mut neg = (0..count)
                .filter(|i| assignment[*i].is_none_or(|v| v.1 < 0.3 && !v.2))
                .collect::<Vec<_>>();
            pos.truncate(128);
            neg.sort_by(|a, b| raw[image * count + b].total_cmp(&raw[image * count + a]));
            neg.truncate(256 - pos.len());
            for anchor in pos {
                selected.push((image * count + anchor) as i64);
                obj_targets.push(1.0);
                positive.push((image * count + anchor) as i64);
                box_targets.extend(encode_box(
                    self.anchors[anchor],
                    coords(&truth[assignment[anchor].unwrap().0].bbox),
                ));
            }
            for anchor in neg {
                selected.push((image * count + anchor) as i64);
                obj_targets.push(0.0);
            }
        }
        let sample_count = selected.len();
        let mut loss = if sample_count > 0 {
            let logits = self
                .objectness
                .reshape([self.batch * count])
                .select(0, indices(selected, &device));
            bce(
                logits,
                Tensor::from_data(BurnData::new(obj_targets, [sample_count]), &device),
            )
            .mean()
        } else {
            self.objectness.sum() * 0.0
        };
        if !positive.is_empty() {
            let n = positive.len();
            loss = loss
                + SmoothL1LossConfig::new()
                    .with_beta(1.0 / 9.0)
                    .init()
                    .forward(
                        self.deltas
                            .reshape([self.batch * count, 4])
                            .select(0, indices(positive, &device)),
                        Tensor::from_data(BurnData::new(box_targets, [n, 4]), &device),
                    )
                    .sum()
                    / sample_count.max(1) as f32;
        }
        let mut labels = Vec::new();
        let mut pos_roi = Vec::new();
        let mut regression = Vec::new();
        let mut masks = Vec::new();
        let mut mask_predictions = Vec::new();
        let mask_size = self.mask_logits.dims()[2];
        for (i, roi) in self.rois.iter().enumerate() {
            let matched = best_match(roi.bounds, &targets[roi.image]);
            if let Some((target, quality)) = matched.filter(|(_, quality)| *quality >= 0.5) {
                let _ = quality;
                let truth = &targets[roi.image][target];
                let class = truth.bbox.class_id;
                labels.push((class + 1) as i64);
                pos_roi.push((i * self.classes + class) as i64);
                regression.extend(encode_box(roi.bounds, coords(&truth.bbox)));
                let full = Tensor::<4>::from_data(
                    BurnData::new(truth.mask.values.clone(), [1, 1, self.height, self.width]),
                    &device,
                );
                masks.push(roi_align(
                    full,
                    &[Roi {
                        image: 0,
                        bounds: roi.bounds,
                    }],
                    mask_size,
                    2,
                )?);
                mask_predictions.push(self.mask_logits.clone().slice([
                    i..i + 1,
                    class..class + 1,
                    0..mask_size,
                    0..mask_size,
                ]));
            } else {
                labels.push(0);
            }
        }
        let roi_count = self.rois.len();
        loss = loss
            + CrossEntropyLossConfig::new().init(&device).forward(
                self.class_logits,
                Tensor::from_data(BurnData::new(labels, [roi_count]), &device),
            );
        if !pos_roi.is_empty() {
            let n = pos_roi.len();
            loss = loss
                + SmoothL1LossConfig::new().init().forward_with_reduction(
                    self.box_deltas
                        .reshape([roi_count * self.classes, 4])
                        .select(0, indices(pos_roi, &device)),
                    Tensor::from_data(BurnData::new(regression, [n, 4]), &device),
                    Reduction::Sum,
                ) / roi_count as f32;
            loss = loss + bce(Tensor::cat(mask_predictions, 0), Tensor::cat(masks, 0)).mean();
        }
        Ok(loss)
    }
    pub fn prediction(self, options: &DetectionOptions) -> Result<MaskRcnnPrediction> {
        options.validate()?;
        let scores = values(softmax(self.class_logits, 1));
        let masks = values(self.mask_logits.clone());
        if scores.iter().chain(&masks).any(|v| !v.is_finite()) {
            return Err(Error::Invalid(
                "Instance heads produced non-finite values".into(),
            ));
        }
        let mask_size = self.mask_logits.dims()[2];
        let mut result = vec![Vec::new(); self.batch];
        for (image, output) in result.iter_mut().enumerate() {
            let mut candidates = Vec::new();
            for (i, _roi) in self
                .rois
                .iter()
                .enumerate()
                .filter(|(_, roi)| roi.image == image)
            {
                if !self.mask_valid[i] {
                    continue;
                }
                let class = (1..=self.classes)
                    .max_by(|a, b| {
                        scores[i * (self.classes + 1) + a]
                            .total_cmp(&scores[i * (self.classes + 1) + b])
                    })
                    .unwrap_or(1);
                let confidence = scores[i * (self.classes + 1) + class];
                if confidence < options.confidence_threshold
                    || confidence <= scores[i * (self.classes + 1)]
                {
                    continue;
                }
                let bounds = self.mask_rois[i].bounds;
                if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
                    continue;
                }
                candidates.push((i, class - 1, confidence, bounds));
            }
            candidates.sort_by(|a, b| b.2.total_cmp(&a.2));
            let mut kept: Vec<(usize, usize, f32, BoxCoords)> = Vec::new();
            for candidate in candidates {
                if kept.iter().any(|old| {
                    old.1 == candidate.1 && iou(old.3, candidate.3) > options.iou_threshold
                }) {
                    continue;
                }
                kept.push(candidate);
                if kept.len() == options.max_detections {
                    break;
                }
            }
            let output_pixels = self
                .height
                .checked_mul(self.width)
                .and_then(|n| n.checked_mul(kept.len()))
                .ok_or_else(|| Error::Invalid("Mask output size overflows".into()))?;
            if output_pixels > 64 * 1024 * 1024 {
                return Err(Error::Invalid(
                    "Instance masks exceed the output element limit".into(),
                ));
            }
            for (i, class, confidence, bounds) in kept {
                let offset = (i * self.classes + class) * mask_size * mask_size;
                let mask = paste_mask(
                    &masks[offset..offset + mask_size * mask_size],
                    mask_size,
                    bounds,
                    self.height,
                    self.width,
                );
                output.push(InstanceDetection {
                    detection: Detection {
                        class_id: class,
                        confidence,
                        x_min: bounds[0],
                        y_min: bounds[1],
                        x_max: bounds[2],
                        y_max: bounds[3],
                    },
                    mask: TensorData {
                        shape: vec![self.height, self.width],
                        values: mask,
                    },
                });
            }
        }
        Ok(MaskRcnnPrediction { instances: result })
    }
}

pub fn roi_align(
    features: Tensor<4>,
    rois: &[Roi],
    size: usize,
    sampling: usize,
) -> Result<Tensor<4>> {
    let [batch, channels, height, width] = features.dims();
    let device = features.device();
    if rois.is_empty()
        || size == 0
        || size > 56
        || sampling == 0
        || sampling > 4
        || height == 0
        || width == 0
    {
        return Err(Error::Invalid("Invalid ROIAlign dimensions".into()));
    }
    let sample_count = size * size * sampling * sampling;
    let mut output = Vec::new();
    for roi in rois {
        if roi.image >= batch || !valid_box(roi.bounds) {
            return Err(Error::Invalid("Invalid ROIAlign box".into()));
        }
        let mut corners: [Vec<i64>; 4] = std::array::from_fn(|_| Vec::with_capacity(sample_count));
        let mut weights: [Vec<f32>; 4] = std::array::from_fn(|_| Vec::with_capacity(sample_count));
        let [x0, y0, x1, y1] = roi.bounds;
        for py in 0..size {
            for px in 0..size {
                for sy in 0..sampling {
                    for sx in 0..sampling {
                        let x = (x0 * width as f32 - 0.5
                            + (px as f32 + (sx as f32 + 0.5) / sampling as f32)
                                * (x1 - x0)
                                * width as f32
                                / size as f32)
                            .clamp(0.0, (width - 1) as f32);
                        let y = (y0 * height as f32 - 0.5
                            + (py as f32 + (sy as f32 + 0.5) / sampling as f32)
                                * (y1 - y0)
                                * height as f32
                                / size as f32)
                            .clamp(0.0, (height - 1) as f32);
                        let left = x.floor() as usize;
                        let top = y.floor() as usize;
                        let right = (left + 1).min(width - 1);
                        let bottom = (top + 1).min(height - 1);
                        let dx = x - left as f32;
                        let dy = y - top as f32;
                        for (c, (index, weight)) in [
                            (top * width + left, (1.0 - dx) * (1.0 - dy)),
                            (top * width + right, dx * (1.0 - dy)),
                            (bottom * width + left, (1.0 - dx) * dy),
                            (bottom * width + right, dx * dy),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            corners[c].push(index as i64);
                            weights[c].push(weight);
                        }
                    }
                }
            }
        }
        let image = features
            .clone()
            .slice([roi.image..roi.image + 1, 0..channels, 0..height, 0..width])
            .reshape([1, channels, height * width]);
        let mut sum = Tensor::<3>::zeros([1, channels, sample_count], &device);
        for (corner, weight) in corners.into_iter().zip(weights) {
            sum = sum
                + image.clone().select(2, indices(corner, &device))
                    * Tensor::<3>::from_data(BurnData::new(weight, [1, 1, sample_count]), &device);
        }
        output.push(
            sum.reshape([1, channels, size, size, sampling * sampling])
                .mean_dim(4)
                .squeeze_dim(4),
        );
    }
    Ok(Tensor::cat(output, 0))
}
fn values<const D: usize>(tensor: Tensor<D>) -> Vec<f32> {
    tensor.into_data().iter::<f32>().collect()
}
fn indices(values: Vec<i64>, device: &Device) -> Tensor<1, Int> {
    let n = values.len();
    Tensor::from_data(BurnData::new(values, [n]), device)
}
fn bce<const D: usize>(logits: Tensor<D>, targets: Tensor<D>) -> Tensor<D> {
    logits.clone().clamp_min(0.0) - logits.clone() * targets + (-logits.abs()).exp().log1p()
}
fn coords(b: &crate::BoundingBox) -> BoxCoords {
    [b.x_min, b.y_min, b.x_max, b.y_max]
}
fn valid_box(b: BoxCoords) -> bool {
    b.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) && b[2] > b[0] && b[3] > b[1]
}
fn validate_targets(
    targets: &[Vec<InstanceTarget>],
    batch: usize,
    classes: usize,
    height: usize,
    width: usize,
) -> Result<()> {
    if targets.len() != batch || targets.iter().any(|v| v.len() > 1024) {
        return Err(Error::Invalid("Invalid instance target batch size".into()));
    }
    let mut elements = 0usize;
    for instances in targets {
        for instance in instances {
            instance.validate(classes, height, width)?;
            elements = elements
                .checked_add(instance.mask.values.len())
                .ok_or_else(|| Error::Invalid("Instance masks exceed limit".into()))?;
        }
    }
    if elements > 64 * 1024 * 1024 {
        return Err(Error::Invalid("Instance masks exceed limit".into()));
    }
    Ok(())
}
fn anchors(
    fh: usize,
    fw: usize,
    height: usize,
    width: usize,
    config: &MaskRcnnConfig,
) -> Vec<BoxCoords> {
    let mut anchors = Vec::new();
    let minimum = height.min(width) as f32;
    for y in 0..fh {
        for x in 0..fw {
            for &scale in &config.anchor_scales {
                for &ratio in &config.anchor_ratios {
                    let w = scale * ratio.sqrt() * minimum / width as f32;
                    let h = scale / ratio.sqrt() * minimum / height as f32;
                    let cx = (x as f32 + 0.5) / fw as f32;
                    let cy = (y as f32 + 0.5) / fh as f32;
                    anchors.push([cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0]);
                }
            }
        }
    }
    anchors
}
fn iou(a: BoxCoords, b: BoxCoords) -> f32 {
    let intersection =
        (a[2].min(b[2]) - a[0].max(b[0])).max(0.0) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    intersection
        / ((a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - intersection + 1e-8)
}
fn encode_box(a: BoxCoords, b: BoxCoords) -> BoxCoords {
    let w = a[2] - a[0];
    let h = a[3] - a[1];
    [
        (b[0] + b[2] - a[0] - a[2]) / (2.0 * w),
        (b[1] + b[3] - a[1] - a[3]) / (2.0 * h),
        ((b[2] - b[0]) / w).ln(),
        ((b[3] - b[1]) / h).ln(),
    ]
}
fn decode_box(a: BoxCoords, d: BoxCoords) -> BoxCoords {
    let w = a[2] - a[0];
    let h = a[3] - a[1];
    let cx = (a[0] + a[2]) / 2.0 + d[0] * w;
    let cy = (a[1] + a[3]) / 2.0 + d[1] * h;
    let w = w * d[2].clamp(-5.0, 5.0).exp();
    let h = h * d[3].clamp(-5.0, 5.0).exp();
    [
        (cx - w / 2.0).clamp(0.0, 1.0),
        (cy - h / 2.0).clamp(0.0, 1.0),
        (cx + w / 2.0).clamp(0.0, 1.0),
        (cy + h / 2.0).clamp(0.0, 1.0),
    ]
}
fn proposals(
    anchors: &[BoxCoords],
    scores: &[f32],
    deltas: &[f32],
    limit: usize,
) -> Result<Vec<BoxCoords>> {
    if scores.iter().chain(deltas).any(|v| !v.is_finite()) {
        return Err(Error::Invalid("RPN produced non-finite values".into()));
    }
    let mut order = (0..anchors.len()).collect::<Vec<_>>();
    order.sort_by(|a, b| scores[*b].total_cmp(&scores[*a]));
    order.truncate((limit * 8).min(2048));
    let mut kept = Vec::new();
    for i in order {
        let bounds = decode_box(anchors[i], deltas[i * 4..i * 4 + 4].try_into().unwrap());
        if !valid_box(bounds) || kept.iter().any(|b| iou(*b, bounds) > 0.7) {
            continue;
        }
        kept.push(bounds);
        if kept.len() == limit {
            break;
        }
    }
    Ok(kept)
}
fn best_match(bounds: BoxCoords, targets: &[InstanceTarget]) -> Option<(usize, f32)> {
    targets
        .iter()
        .enumerate()
        .map(|(i, t)| (i, iou(bounds, coords(&t.bbox))))
        .max_by(|a, b| a.1.total_cmp(&b.1))
}
fn sample_rois(
    proposals: Vec<BoxCoords>,
    targets: &[InstanceTarget],
    limit: usize,
    seed: u64,
) -> Vec<BoxCoords> {
    let mut positive = Vec::new();
    let mut negative = Vec::new();
    for bounds in proposals {
        let score = best_match(bounds, targets).map_or(0.0, |(_, q)| q);
        if score >= 0.5 {
            positive.push(bounds);
        } else {
            negative.push(bounds);
        }
    }
    // Ground-truth boxes have perfect IoU; ranking by IoU would exclude useful regression examples.
    let mut result = crate::engine::shuffled_indices(positive.len(), seed)
        .into_iter()
        .take(limit / 4)
        .map(|index| positive[index])
        .collect::<Vec<_>>();
    negative.truncate(limit - result.len());
    result.extend(negative);
    result
}
fn assign_anchors(
    anchors: &[BoxCoords],
    targets: &[InstanceTarget],
) -> Vec<Option<(usize, f32, bool)>> {
    let mut matches = anchors
        .iter()
        .map(|a| best_match(*a, targets).map(|(i, q)| (i, q, false)))
        .collect::<Vec<_>>();
    for (target, truth) in targets.iter().enumerate() {
        if let Some((best, quality)) = anchors
            .iter()
            .enumerate()
            .map(|(i, b)| (i, iou(*b, coords(&truth.bbox))))
            .max_by(|a, b| a.1.total_cmp(&b.1))
        {
            matches[best] = Some((target, quality, true));
        }
    }
    matches
}
fn paste_mask(
    mask: &[f32],
    size: usize,
    bounds: BoxCoords,
    height: usize,
    width: usize,
) -> Vec<f32> {
    let mut result = vec![0.0; height * width];
    for y in 0..height {
        for x in 0..width {
            let px = (x as f32 + 0.5) / width as f32;
            let py = (y as f32 + 0.5) / height as f32;
            if px < bounds[0] || px > bounds[2] || py < bounds[1] || py > bounds[3] {
                continue;
            }
            let sx = ((px - bounds[0]) / (bounds[2] - bounds[0]) * size as f32 - 0.5)
                .clamp(0.0, (size - 1) as f32);
            let sy = ((py - bounds[1]) / (bounds[3] - bounds[1]) * size as f32 - 0.5)
                .clamp(0.0, (size - 1) as f32);
            let x0 = sx.floor() as usize;
            let y0 = sy.floor() as usize;
            let x1 = (x0 + 1).min(size - 1);
            let y1 = (y0 + 1).min(size - 1);
            let dx = sx - x0 as f32;
            let dy = sy - y0 as f32;
            let value = mask[y0 * size + x0] * (1.0 - dx) * (1.0 - dy)
                + mask[y0 * size + x1] * dx * (1.0 - dy)
                + mask[y1 * size + x0] * (1.0 - dx) * dy
                + mask[y1 * size + x1] * dx * dy;
            result[y * width + x] = if value > 0.0 { 1.0 } else { 0.0 };
        }
    }
    result
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;
    use burn::optim::{AdamConfig, GradientsParams};
    #[test]
    fn roi_align_has_known_geometry_and_gradients() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = Device::flex().autodiff();
        let input = Tensor::<4>::from_data(
            BurnData::new((0..16).map(|v| v as f32).collect::<Vec<_>>(), [1, 1, 4, 4]),
            &device,
        )
        .require_grad();
        let output = roi_align(
            input.clone(),
            &[Roi {
                image: 0,
                bounds: [0.0, 0.0, 1.0, 1.0],
            }],
            2,
            2,
        )
        .unwrap();
        assert_eq!(values(output.clone()), vec![2.5, 4.5, 10.5, 12.5]);
        let gradients = output.sum().backward();
        let gradient = values(input.grad(&gradients).unwrap());
        assert!(gradient.iter().all(|v| (*v - 0.25).abs() < 1e-6));
    }
    #[test]
    fn box_deltas_roundtrip_and_anchor_assignment_keeps_truth() {
        let a = [0.1, 0.1, 0.5, 0.5];
        let b = [0.2, 0.15, 0.6, 0.7];
        for (x, y) in decode_box(a, encode_box(a, b)).iter().zip(b) {
            assert!((x - y).abs() < 1e-6);
        }
        let truth = InstanceTarget {
            bbox: crate::BoundingBox {
                class_id: 0,
                x_min: 0.4,
                y_min: 0.4,
                x_max: 0.5,
                y_max: 0.5,
            },
            mask: TensorData {
                shape: vec![1, 1],
                values: vec![1.0],
            },
        };
        let assigned = assign_anchors(&[[0.0, 0.0, 0.3, 0.3], [0.2, 0.2, 0.8, 0.8]], &[truth]);
        assert!(
            assigned[1]
                .is_some_and(|(index, quality, forced)| index == 0 && quality < 0.3 && forced)
        );
    }
    #[test]
    fn positive_roi_sampling_keeps_imperfect_regression_examples() {
        let truth = InstanceTarget {
            bbox: crate::BoundingBox {
                class_id: 0,
                x_min: 0.2,
                y_min: 0.2,
                x_max: 0.8,
                y_max: 0.8,
            },
            mask: TensorData {
                shape: vec![1, 1],
                values: vec![1.0],
            },
        };
        let exact = coords(&truth.bbox);
        let imperfect = [0.15, 0.2, 0.75, 0.8];
        assert!(iou(imperfect, exact) > 0.5);
        let mut selected_imperfect = false;
        let mut selected_exact = false;
        for seed in 0..32 {
            let proposals = vec![imperfect, exact];
            let selected = sample_rois(proposals.clone(), std::slice::from_ref(&truth), 4, seed);
            assert_eq!(selected.len(), 1);
            assert_eq!(
                selected,
                sample_rois(proposals, std::slice::from_ref(&truth), 4, seed),
                "Restoring the sampling seed must reproduce the same proposals"
            );
            selected_imperfect |= encode_box(selected[0], exact) != [0.0; 4];
            selected_exact |= selected[0] == exact;
        }
        assert!(
            selected_imperfect,
            "Box refinement needs nonzero regression targets"
        );
        assert!(
            selected_exact,
            "Ground-truth proposals must remain eligible"
        );
    }
    #[test]
    fn mask_rcnn_learns_instances_and_reloads() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = Device::flex().autodiff();
        device.seed(42);
        let config = MaskRcnnConfig {
            input_channels: 1,
            classes: 1,
            base_channels: 4,
            anchor_scales: vec![0.5],
            anchor_ratios: vec![1.0],
            roi_size: 2,
            mask_size: 4,
            proposals: 8,
            training_samples: 8,
        };
        let mut model = MaskRcnn::new(&config, &device).unwrap();
        let mut optimizer = AdamConfig::new().init();
        let pixels = (0..256)
            .map(|i| {
                if (4..12).contains(&(i / 16)) && (4..12).contains(&(i % 16)) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect::<Vec<_>>();
        let input =
            || Tensor::<4>::from_data(BurnData::new(pixels.clone(), [1, 1, 16, 16]), &device);
        let truth = vec![vec![InstanceTarget {
            bbox: crate::BoundingBox {
                class_id: 0,
                x_min: 0.25,
                y_min: 0.25,
                x_max: 0.75,
                y_max: 0.75,
            },
            mask: TensorData {
                shape: vec![16, 16],
                values: pixels.clone(),
            },
        }]];
        let initial = values(
            model
                .forward(input(), Some(&truth))
                .unwrap()
                .loss(&truth)
                .unwrap(),
        )[0];
        let mut last = initial;
        for _ in 0..100 {
            let loss = model
                .forward(input(), Some(&truth))
                .unwrap()
                .loss(&truth)
                .unwrap();
            last = values(loss.clone())[0];
            let gradients = GradientsParams::from_grads(loss.backward(), &model);
            model = optimizer.step(0.003, model, gradients);
        }
        assert!(
            last < initial * 0.8,
            "Mask R-CNN loss did not learn: {initial} -> {last}"
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mask.bpk");
        model.clone().save_file(&path).unwrap();
        let loaded = MaskRcnn::new(&config, &device)
            .unwrap()
            .try_load_file(&path)
            .unwrap();
        let first = values(model.forward(input(), None).unwrap().class_logits);
        let second = values(loaded.forward(input(), None).unwrap().class_logits);
        assert_eq!(first, second);
        let prediction = model
            .forward(input(), None)
            .unwrap()
            .prediction(&DetectionOptions {
                confidence_threshold: 0.0,
                iou_threshold: 0.5,
                max_detections: 8,
            })
            .unwrap();
        assert_eq!(prediction.instances.len(), 1);
        let best_iou = prediction.instances[0]
            .iter()
            .map(|instance| {
                let intersection = instance
                    .mask
                    .values
                    .iter()
                    .zip(&pixels)
                    .filter(|(a, b)| **a > 0.5 && **b > 0.5)
                    .count();
                let union = instance
                    .mask
                    .values
                    .iter()
                    .zip(&pixels)
                    .filter(|(a, b)| **a > 0.5 || **b > 0.5)
                    .count();
                intersection as f32 / union.max(1) as f32
            })
            .fold(0.0, f32::max);
        assert!(
            best_iou > 0.5,
            "Trained instance masks did not localize the object: IoU {best_iou}"
        );
    }

    #[test]
    fn generic_training_persists_instance_outputs_and_empty_targets_have_finite_loss() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let config = MaskRcnnConfig {
            input_channels: 1,
            classes: 1,
            base_channels: 2,
            anchor_scales: vec![0.5],
            anchor_ratios: vec![1.0],
            roi_size: 2,
            mask_size: 4,
            proposals: 8,
            training_samples: 8,
        };
        let pixels = (0..256)
            .map(|i| {
                if (4..12).contains(&(i / 16)) && (4..12).contains(&(i % 16)) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect::<Vec<_>>();
        let inputs = TensorData {
            shape: vec![1, 1, 16, 16],
            values: pixels.clone(),
        };
        let dataset = crate::TensorDataset {
            inputs: inputs.clone(),
            targets: crate::Targets::Instances {
                values: vec![vec![InstanceTarget {
                    bbox: crate::BoundingBox {
                        class_id: 0,
                        x_min: 0.25,
                        y_min: 0.25,
                        x_max: 0.75,
                        y_max: 0.75,
                    },
                    mask: TensorData {
                        shape: vec![16, 16],
                        values: pixels,
                    },
                }]],
            },
        };
        let training = crate::TrainingConfig {
            recipe: crate::Recipe::MaskRcnn {
                config: config.clone(),
            },
            backend: crate::BackendChoice::Cpu,
            epochs: 2,
            batch_size: 1,
            learning_rate: 0.01,
            seed: 43,
            gradient_clip: 5.0,
        };
        let directory = tempfile::tempdir().unwrap();
        let report = crate::train(
            &training,
            &dataset,
            None,
            directory.path(),
            &crate::CancellationToken::new(),
            |_| {},
        )
        .unwrap();
        assert!(report.final_loss.is_finite());
        let first = crate::Predictor::load(directory.path(), crate::BackendChoice::Cpu)
            .unwrap()
            .predict(&inputs)
            .unwrap();
        let second = crate::Predictor::load(directory.path(), crate::BackendChoice::Cpu)
            .unwrap()
            .predict(&inputs)
            .unwrap();
        assert_eq!(
            serde_json::to_value(&first.instances).unwrap(),
            serde_json::to_value(&second.instances).unwrap()
        );
        assert_eq!(first.instances.unwrap().len(), 1);
        let device = Device::flex().autodiff();
        let model = MaskRcnn::new(&config, &device).unwrap();
        let empty = vec![vec![]];
        let tensor = Tensor::<4>::from_data(BurnData::new(inputs.values, [1, 1, 16, 16]), &device);
        let loss = model
            .forward(tensor, Some(&empty))
            .unwrap()
            .loss(&empty)
            .unwrap();
        assert!(values(loss)[0].is_finite());
    }
}

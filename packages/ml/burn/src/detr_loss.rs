use crate::{BoundingBox, Error, Result};
use burn::tensor::{Int, Tensor, TensorData, activation};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetrClassificationLoss {
    #[default]
    Varifocal,
    IouAwareBce,
}

/// Matching costs and loss weights for sigmoid DETR detection heads.
#[derive(Clone, Debug)]
pub struct DetrLossConfig {
    pub classification: DetrClassificationLoss,
    pub match_class: f64,
    pub match_l1: f64,
    pub match_giou: f64,
    pub class_weight: f64,
    pub l1_weight: f64,
    pub giou_weight: f64,
    pub focal_alpha: f64,
    pub focal_gamma: f64,
    pub varifocal_alpha: f64,
    pub varifocal_gamma: f64,
}

impl Default for DetrLossConfig {
    fn default() -> Self {
        Self {
            classification: DetrClassificationLoss::Varifocal,
            match_class: 2.0,
            match_l1: 5.0,
            match_giou: 2.0,
            class_weight: 1.0,
            l1_weight: 5.0,
            giou_weight: 2.0,
            focal_alpha: 0.25,
            focal_gamma: 2.0,
            varifocal_alpha: 0.75,
            varifocal_gamma: 2.0,
        }
    }
}

impl DetrLossConfig {
    fn validate(&self) -> Result<()> {
        if [
            self.match_class,
            self.match_l1,
            self.match_giou,
            self.class_weight,
            self.l1_weight,
            self.giou_weight,
            self.focal_gamma,
            self.varifocal_gamma,
        ]
        .into_iter()
        .any(|value| !value.is_finite() || !(0.0..=1e6).contains(&value))
            || [self.focal_alpha, self.varifocal_alpha]
                .into_iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
            || self.match_class + self.match_l1 + self.match_giou == 0.0
            || self.class_weight + self.l1_weight + self.giou_weight == 0.0
        {
            return Err(Error::Invalid("Invalid DETR loss configuration".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetrMatch {
    pub query: usize,
    pub target: usize,
}

struct Predictions {
    batch: usize,
    queries: usize,
    classes: usize,
    logits: Vec<f32>,
    boxes: Vec<f32>,
}

impl Predictions {
    fn read(
        logits: &Tensor<3>,
        boxes: &Tensor<3>,
        targets: &[Vec<BoundingBox>],
        config: &DetrLossConfig,
    ) -> Result<Self> {
        config.validate()?;
        let [batch, queries, classes] = logits.dims();
        if batch == 0
            || queries == 0
            || classes == 0
            || boxes.dims() != [batch, queries, 4]
            || targets.len() != batch
            || logits.device() != boxes.device()
        {
            return Err(Error::Invalid(
                "DETR needs logits [N,Q,C], boxes [N,Q,4] and N target lists on one device".into(),
            ));
        }
        for image_targets in targets {
            for target in image_targets {
                if target.class_id >= classes
                    || [target.x_min, target.y_min, target.x_max, target.y_max]
                        .into_iter()
                        .any(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
                    || target.x_min >= target.x_max
                    || target.y_min >= target.y_max
                {
                    return Err(Error::Invalid(
                        "DETR targets need valid classes and nonempty normalized boxes".into(),
                    ));
                }
            }
        }
        let logits = logits.clone().into_data().iter::<f32>().collect::<Vec<_>>();
        let boxes = boxes.clone().into_data().iter::<f32>().collect::<Vec<_>>();
        let maximum_coordinate = f32::MAX.sqrt() * 0.125;
        if logits.iter().any(|value| !value.is_finite())
            || boxes
                .iter()
                .any(|value| !value.is_finite() || value.abs() > maximum_coordinate)
            || boxes
                .chunks_exact(4)
                .any(|value| value[2] < 0.0 || value[3] < 0.0)
        {
            return Err(Error::Invalid(
                "DETR predictions need finite logits and center/size boxes with nonnegative sizes and finite area arithmetic"
                    .into(),
            ));
        }
        Ok(Self {
            batch,
            queries,
            classes,
            logits,
            boxes,
        })
    }

    fn box_at(&self, batch: usize, query: usize) -> [f64; 4] {
        let start = (batch * self.queries + query) * 4;
        std::array::from_fn(|i| self.boxes[start + i] as f64)
    }

    fn matches(
        &self,
        targets: &[Vec<BoundingBox>],
        config: &DetrLossConfig,
    ) -> Result<Vec<Vec<DetrMatch>>> {
        targets
            .iter()
            .enumerate()
            .map(|(batch, targets)| {
                if targets.is_empty() {
                    return Ok(Vec::new());
                }
                let transpose = targets.len() > self.queries;
                let rows = targets.len().min(self.queries);
                let columns = targets.len().max(self.queries);
                let cells = rows.checked_mul(columns).ok_or_else(|| {
                    Error::Invalid("DETR assignment matrix dimensions overflow".into())
                })?;
                if cells > 16 * 1024 * 1024 {
                    return Err(Error::Invalid("DETR assignment matrix is too large".into()));
                }
                let mut costs = vec![0.0; cells];
                for (target_index, target) in targets.iter().enumerate() {
                    let target_box = target_cxcywh(target);
                    for query in 0..self.queries {
                        let offset = (batch * self.queries + query) * self.classes;
                        let probability = sigmoid(self.logits[offset + target.class_id] as f64);
                        let positive = config.focal_alpha
                            * (1.0 - probability).powf(config.focal_gamma)
                            * -(probability + 1e-8).ln();
                        let negative = (1.0 - config.focal_alpha)
                            * probability.powf(config.focal_gamma)
                            * -(1.0 - probability + 1e-8).ln();
                        let predicted = self.box_at(batch, query);
                        let l1 = predicted
                            .iter()
                            .zip(target_box)
                            .map(|(a, b)| (a - b).abs())
                            .sum::<f64>();
                        let (_, giou) = overlap(predicted, target_box);
                        let cost = config.match_class * (positive - negative)
                            + config.match_l1 * l1
                            - config.match_giou * giou;
                        let index = if transpose {
                            query * columns + target_index
                        } else {
                            target_index * columns + query
                        };
                        costs[index] = cost;
                    }
                }
                let mut matched = assignment(&costs, rows, columns)
                    .into_iter()
                    .map(|(row, column)| {
                        if transpose {
                            DetrMatch {
                                query: row,
                                target: column,
                            }
                        } else {
                            DetrMatch {
                                query: column,
                                target: row,
                            }
                        }
                    })
                    .collect::<Vec<_>>();
                matched.sort_unstable_by_key(|pair| pair.query);
                Ok(matched)
            })
            .collect()
    }
}

/// Match each image independently using detached focal, L1 and generalized IoU costs.
/// A target and a query each occur at most once in the returned assignment.
pub fn detr_match(
    logits: &Tensor<3>,
    boxes: &Tensor<3>,
    targets: &[Vec<BoundingBox>],
    config: &DetrLossConfig,
) -> Result<Vec<Vec<DetrMatch>>> {
    Predictions::read(logits, boxes, targets, config)?.matches(targets, config)
}

/// Varifocal classification and matched L1/GIoU losses for one decoder layer.
/// Boxes use normalized center-x, center-y, width, height; targets use corner coordinates.
pub fn detr_loss(
    logits: Tensor<3>,
    boxes: Tensor<3>,
    targets: &[Vec<BoundingBox>],
    config: &DetrLossConfig,
) -> Result<Tensor<1>> {
    let predictions = Predictions::read(&logits, &boxes, targets, config)?;
    let matched = predictions.matches(targets, config)?;
    let normalizer = targets.iter().map(Vec::len).sum::<usize>().max(1) as f64;
    let mut scores = vec![0.0f32; predictions.logits.len()];
    let mut positive_mask = vec![0.0f32; predictions.logits.len()];
    let mut weights = predictions
        .logits
        .iter()
        .map(|&logit| {
            (config.varifocal_alpha * sigmoid(logit as f64).powf(config.varifocal_gamma)) as f32
        })
        .collect::<Vec<_>>();
    let mut indices = Vec::new();
    let mut target_boxes = Vec::new();
    for (batch, matches) in matched.iter().enumerate() {
        for pair in matches {
            let target = &targets[batch][pair.target];
            let target_box = target_cxcywh(target);
            let (iou, _) = overlap(predictions.box_at(batch, pair.query), target_box);
            let index = batch * predictions.queries + pair.query;
            let class_index = index * predictions.classes + target.class_id;
            let score = match config.classification {
                DetrClassificationLoss::Varifocal => iou,
                DetrClassificationLoss::IouAwareBce => {
                    (sigmoid(predictions.logits[class_index] as f64).powf(config.focal_alpha)
                        * iou.powf(1.0 - config.focal_alpha))
                    .max(0.01)
                }
            };
            scores[class_index] = score as f32;
            weights[class_index] = iou as f32;
            positive_mask[class_index] = 1.0;
            indices.push(index as i64);
            target_boxes.extend(target_box.map(|value| value as f32));
        }
    }
    let device = logits.device();
    let shape = [predictions.batch, predictions.queries, predictions.classes];
    let scores = Tensor::<3>::from_data(TensorData::new(scores, shape), &device);
    // The dedicated primitive keeps the correct derivative at zero logits as well
    // as stable values for saturated predictions.
    let positive_bce = -activation::log_sigmoid(logits.clone());
    let negative_bce = -activation::log_sigmoid(-logits.clone());
    let classification = match config.classification {
        DetrClassificationLoss::Varifocal => {
            let weights = Tensor::<3>::from_data(TensorData::new(weights, shape), &device);
            let bce = positive_bce * scores.clone() + negative_bce * (-scores + 1.0);
            (bce * weights).sum()
        }
        DetrClassificationLoss::IouAwareBce => {
            let mask = Tensor::<3>::from_data(TensorData::new(positive_mask, shape), &device);
            // RF-DETR keeps gradients through the unmatched negative probabilities.
            let negative = activation::sigmoid(logits.clone()).powf_scalar(config.focal_gamma)
                * (mask.clone() * -1.0 + 1.0)
                + mask * (scores.clone() * -1.0 + 1.0);
            (scores * positive_bce + negative * negative_bce).sum()
        }
    } * (config.class_weight / normalizer);
    if indices.is_empty() {
        return Ok(classification);
    }
    let count = indices.len();
    let indices = Tensor::<1, Int>::from_data(TensorData::new(indices, [count]), &device);
    let predicted = boxes
        .reshape([predictions.batch * predictions.queries, 4])
        .select(0, indices);
    let target = Tensor::<2>::from_data(TensorData::new(target_boxes, [count, 4]), &device);
    let l1 = (predicted.clone() - target.clone()).abs().sum();
    let giou = (generalized_iou(predicted, target) * -1.0 + 1.0).sum();
    Ok(classification
        + l1 * (config.l1_weight / normalizer)
        + giou * (config.giou_weight / normalizer))
}

fn target_cxcywh(target: &BoundingBox) -> [f64; 4] {
    let [left, top, right, bottom] =
        [target.x_min, target.y_min, target.x_max, target.y_max].map(|value| value as f64);
    [
        (left + right) * 0.5,
        (top + bottom) * 0.5,
        right - left,
        bottom - top,
    ]
}

fn corners([x, y, width, height]: [f64; 4]) -> [f64; 4] {
    [
        x - width * 0.5,
        y - height * 0.5,
        x + width * 0.5,
        y + height * 0.5,
    ]
}

fn overlap(a: [f64; 4], b: [f64; 4]) -> (f64, f64) {
    let area_a = a[2] * a[3];
    let area_b = b[2] * b[3];
    let a = corners(a);
    let b = corners(b);
    let intersection =
        (a[2].min(b[2]) - a[0].max(b[0])).max(0.0) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let union = area_a + area_b - intersection;
    let enclosing = (a[2].max(b[2]) - a[0].min(b[0])) * (a[3].max(b[3]) - a[1].min(b[1]));
    let minimum_area = f32::MIN_POSITIVE as f64;
    let iou = intersection / union.max(minimum_area);
    (iou, iou - (enclosing - union) / enclosing.max(minimum_area))
}

fn generalized_iou(a: Tensor<2>, b: Tensor<2>) -> Tensor<1> {
    let count = a.dims()[0];
    let a_size = a.clone().slice([0..count, 2..4]);
    let b_size = b.clone().slice([0..count, 2..4]);
    let a_min = a.slice([0..count, 0..2]) - a_size.clone() * 0.5;
    let b_min = b.slice([0..count, 0..2]) - b_size.clone() * 0.5;
    let a_max = a_min.clone() + a_size.clone();
    let b_max = b_min.clone() + b_size.clone();
    let intersection_size = (a_max.clone().min_pair(b_max.clone())
        - a_min.clone().max_pair(b_min.clone()))
    .clamp_min(0.0);
    let area =
        |size: Tensor<2>| size.clone().slice([0..count, 0..1]) * size.slice([0..count, 1..2]);
    let intersection = area(intersection_size);
    let union = area(a_size) + area(b_size) - intersection.clone();
    let enclosing = area(a_max.max_pair(b_max) - a_min.min_pair(b_min));
    (intersection / union.clone().clamp_min(f32::MIN_POSITIVE)
        - (enclosing.clone() - union) / enclosing.clamp_min(f32::MIN_POSITIVE))
    .squeeze_dim(1)
}

fn sigmoid(value: f64) -> f64 {
    if value >= 0.0 {
        1.0 / (1.0 + (-value).exp())
    } else {
        let exponential = value.exp();
        exponential / (1.0 + exponential)
    }
}

// Shortest augmenting paths with dual potentials. Rows never outnumber columns.
fn assignment(costs: &[f64], rows: usize, columns: usize) -> Vec<(usize, usize)> {
    let mut row_potential = vec![0.0; rows + 1];
    let mut column_potential = vec![0.0; columns + 1];
    let mut column_row = vec![0usize; columns + 1];
    let mut predecessor = vec![0usize; columns + 1];
    for row in 1..=rows {
        column_row[0] = row;
        let mut distance = vec![f64::INFINITY; columns + 1];
        let mut visited = vec![false; columns + 1];
        let mut column = 0;
        loop {
            visited[column] = true;
            let active_row = column_row[column];
            let mut next_column = 0;
            let mut delta = f64::INFINITY;
            for candidate in 1..=columns {
                if visited[candidate] {
                    continue;
                }
                let reduced = costs[(active_row - 1) * columns + candidate - 1]
                    - row_potential[active_row]
                    - column_potential[candidate];
                if reduced < distance[candidate] {
                    distance[candidate] = reduced;
                    predecessor[candidate] = column;
                }
                if distance[candidate] < delta {
                    delta = distance[candidate];
                    next_column = candidate;
                }
            }
            for candidate in 0..=columns {
                if visited[candidate] {
                    row_potential[column_row[candidate]] += delta;
                    column_potential[candidate] -= delta;
                } else {
                    distance[candidate] -= delta;
                }
            }
            column = next_column;
            if column_row[column] == 0 {
                break;
            }
        }
        loop {
            let previous = predecessor[column];
            column_row[column] = column_row[previous];
            column = previous;
            if column == 0 {
                break;
            }
        }
    }
    (1..=columns)
        .filter_map(|column| {
            (column_row[column] != 0).then_some((column_row[column].wrapping_sub(1), column - 1))
        })
        .collect()
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;

    fn target(class_id: usize, x_min: f32, x_max: f32) -> BoundingBox {
        BoundingBox {
            class_id,
            x_min,
            y_min: 0.2,
            x_max,
            y_max: 0.6,
        }
    }

    #[test]
    fn assignment_finds_global_rectangular_optimum_including_negative_costs() {
        let costs = [0.0, 1.0, 20.0, -2.0, 10.0, 20.0];
        let pairs = assignment(&costs, 2, 3);
        assert_eq!(pairs, vec![(1, 0), (0, 1)]);
        assert_eq!(
            pairs.iter().map(|&(r, c)| costs[r * 3 + c]).sum::<f64>(),
            -1.0
        );
        assert_eq!(assignment(&[0.0; 6], 2, 3).len(), 2);
    }

    #[test]
    fn matching_respects_classes_geometry_and_more_targets_than_queries() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        let logits = Tensor::from_data([[[-6.0f32, 6.0], [6.0, -6.0]]], &device);
        let boxes = Tensor::from_data([[[0.7f32, 0.4, 0.2, 0.4], [0.2, 0.4, 0.2, 0.4]]], &device);
        let mut targets = vec![vec![target(0, 0.1, 0.3), target(1, 0.6, 0.8)]];
        let config = DetrLossConfig::default();
        let expected = vec![vec![
            DetrMatch {
                query: 0,
                target: 1,
            },
            DetrMatch {
                query: 1,
                target: 0,
            },
        ]];
        assert_eq!(
            detr_match(&logits, &boxes, &targets, &config).unwrap(),
            expected
        );
        targets[0].push(target(1, 0.1, 0.9));
        assert_eq!(
            detr_match(&logits, &boxes, &targets, &config).unwrap(),
            expected
        );
    }

    #[test]
    fn varifocal_quality_and_empty_image_loss_match_scalar_reference() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        let logits = Tensor::from_data([[[0.0f32]]], &device);
        let boxes = Tensor::from_data([[[0.5f32, 0.5, 0.5, 0.5]]], &device);
        let target = BoundingBox {
            class_id: 0,
            x_min: 0.25,
            y_min: 0.25,
            x_max: 0.75,
            y_max: 0.75,
        };
        let config = DetrLossConfig::default();
        let value = detr_loss(logits.clone(), boxes.clone(), &[vec![target]], &config)
            .unwrap()
            .into_scalar::<f32>();
        assert!((value - std::f32::consts::LN_2).abs() < 1e-6);
        let empty = detr_loss(logits, boxes, &[vec![]], &config)
            .unwrap()
            .into_scalar::<f32>();
        assert!((empty - 0.75 * 0.25 * std::f32::consts::LN_2).abs() < 1e-6);
    }

    #[test]
    fn zero_logit_classification_gradients_match_analytic_values() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        for (classification, matched, expected_loss, expected_gradient) in [
            (
                DetrClassificationLoss::Varifocal,
                true,
                std::f64::consts::LN_2,
                -0.5,
            ),
            (
                DetrClassificationLoss::Varifocal,
                false,
                0.1875 * std::f64::consts::LN_2,
                0.09375,
            ),
            (
                DetrClassificationLoss::IouAwareBce,
                true,
                std::f64::consts::LN_2,
                0.5 - 0.5f64.powf(0.25),
            ),
            (
                DetrClassificationLoss::IouAwareBce,
                false,
                0.25 * std::f64::consts::LN_2,
                0.125 + 0.25 * std::f64::consts::LN_2,
            ),
        ] {
            let logits = Tensor::from_data([[[0.0f32]]], &device).require_grad();
            let boxes = Tensor::from_data([[[0.5f32, 0.5, 0.5, 0.5]]], &device);
            let targets = if matched {
                vec![BoundingBox {
                    class_id: 0,
                    x_min: 0.25,
                    y_min: 0.25,
                    x_max: 0.75,
                    y_max: 0.75,
                }]
            } else {
                Vec::new()
            };
            let config = DetrLossConfig {
                classification,
                ..Default::default()
            };
            let loss = detr_loss(logits.clone(), boxes, &[targets], &config).unwrap();
            let actual_loss = loss.clone().into_scalar::<f32>() as f64;
            let actual_gradient =
                logits.grad(&loss.backward()).unwrap().into_scalar::<f32>() as f64;
            assert!(
                (actual_loss - expected_loss).abs() < 1e-6,
                "{classification:?}, matched={matched}: loss {actual_loss} != {expected_loss}"
            );
            assert!(
                (actual_gradient - expected_gradient).abs() < 1e-6,
                "{classification:?}, matched={matched}: gradient {actual_gradient} != {expected_gradient}"
            );
        }
    }

    #[test]
    fn matched_class_and_box_losses_have_finite_nonzero_gradients() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        let logits = Tensor::from_data([[[0.5f32, -0.5]]], &device).require_grad();
        let boxes = Tensor::from_data([[[0.35f32, 0.45, 0.3, 0.3]]], &device).require_grad();
        let gradients = detr_loss(
            logits.clone(),
            boxes.clone(),
            &[vec![target(0, 0.1, 0.3)]],
            &DetrLossConfig::default(),
        )
        .unwrap()
        .backward();
        for values in [
            logits
                .grad(&gradients)
                .unwrap()
                .into_data()
                .iter::<f32>()
                .collect::<Vec<_>>(),
            boxes
                .grad(&gradients)
                .unwrap()
                .into_data()
                .iter::<f32>()
                .collect::<Vec<_>>(),
        ] {
            assert!(values.iter().all(|value| value.is_finite()));
            assert!(values.iter().any(|value| value.abs() > 1e-6));
        }
    }

    #[test]
    fn iou_aware_bce_keeps_negative_probability_gradients_and_detaches_positive_quality() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        let config = DetrLossConfig {
            classification: DetrClassificationLoss::IouAwareBce,
            ..Default::default()
        };
        let boxes = Tensor::from_data([[[1.1f32, 0.5, 0.5, 0.5]]], &device);
        let logits = Tensor::from_data([[[0.5f32]]], &device).require_grad();
        let loss = detr_loss(logits.clone(), boxes, &[vec![]], &config).unwrap();
        let probability = sigmoid(0.5);
        let softplus = 0.5f64.exp().ln_1p();
        assert!(
            (loss.clone().into_scalar::<f32>() as f64 - probability.powi(2) * softplus).abs()
                < 1e-6
        );
        let gradient = logits.grad(&loss.backward()).unwrap().into_scalar::<f32>() as f64;
        let expected =
            2.0 * probability.powi(2) * (1.0 - probability) * softplus + probability.powi(3);
        assert!((gradient - expected).abs() < 1e-6);

        let boxes = Tensor::from_data([[[0.5f32, 0.5, 0.5, 0.5]]], &device);
        let logits = Tensor::from_data([[[0.5f32]]], &device).require_grad();
        let target = BoundingBox {
            class_id: 0,
            x_min: 0.25,
            y_min: 0.25,
            x_max: 0.75,
            y_max: 0.75,
        };
        let loss = detr_loss(logits.clone(), boxes, &[vec![target]], &config).unwrap();
        let quality = probability.powf(0.25);
        let expected = quality * (-0.5f64).exp().ln_1p() + (1.0 - quality) * softplus;
        assert!((loss.clone().into_scalar::<f32>() as f64 - expected).abs() < 1e-6);
        let gradient = logits.grad(&loss.backward()).unwrap().into_scalar::<f32>() as f64;
        assert!((gradient - (probability - quality)).abs() < 1e-6);
    }

    #[test]
    fn rejects_nonfinite_predictions_invalid_targets_and_batch_mismatch() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        let logits = Tensor::from_data([[[0.0f32]]], &device);
        let boxes = Tensor::from_data([[[0.5f32, 0.5, 0.2, 0.2]]], &device);
        let config = DetrLossConfig::default();
        for targets in [
            vec![],
            vec![vec![target(1, 0.1, 0.3)]],
            vec![vec![target(0, -0.1, 0.3)]],
            vec![vec![target(0, 0.3, 0.3)]],
        ] {
            assert!(detr_loss(logits.clone(), boxes.clone(), &targets, &config).is_err());
        }
        let invalid = Tensor::from_data([[[f32::NAN]]], &device);
        assert!(detr_loss(invalid, boxes, &[vec![]], &config).is_err());
        let invalid_box = Tensor::from_data([[[0.5f32, 0.5, -0.2, 0.2]]], &device);
        assert!(detr_loss(logits, invalid_box, &[vec![]], &config).is_err());
    }
}

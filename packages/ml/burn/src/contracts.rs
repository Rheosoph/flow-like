use crate::{Error, Objective, Recipe, Result, TrainingConfig};
use flow_like_ml_core::{InspectionSpec, TaskKind};

/// Validate the inspection task and tensor contract before a training job consumes its trigger.
pub fn validate_inspection_config(spec: &InspectionSpec, config: &TrainingConfig) -> Result<()> {
    spec.validate().map_err(|e| Error::Invalid(e.to_string()))?;
    config.validate()?;
    let classification = config.recipe.objective() == Some(Objective::Classification);
    let regression = config.recipe.objective() == Some(Objective::Regression);
    let sensor = matches!(
        config.recipe,
        Recipe::Mlp { .. }
            | Recipe::Lstm { .. }
            | Recipe::Gru { .. }
            | Recipe::Cnn1d { .. }
            | Recipe::Tcn { .. }
    );
    let reconstruction = matches!(
        config.recipe,
        Recipe::DenseAutoencoder { .. }
            | Recipe::Conv1dAutoencoder { .. }
            | Recipe::LstmAutoencoder { .. }
    );
    let compatible = match spec.task {
        TaskKind::ImageClassification => matches!(
            config.recipe,
            Recipe::ResNet18 { .. } | Recipe::MobileNetV2 { .. } | Recipe::EfficientNet { .. }
        ),
        TaskKind::ObjectDetection => matches!(config.recipe, Recipe::YoloX { .. }),
        TaskKind::Segmentation => matches!(config.recipe, Recipe::UNet { .. }),
        TaskKind::InstanceSegmentation => matches!(config.recipe, Recipe::MaskRcnn { .. }),
        TaskKind::VisualAnomaly => matches!(config.recipe, Recipe::DenseAutoencoder { .. }),
        TaskKind::SensorClassification => sensor && classification,
        TaskKind::SensorRegression => sensor && regression && config.recipe.outputs() == 1,
        TaskKind::SensorAnomaly => reconstruction,
        TaskKind::SequenceAutoencoder => matches!(
            config.recipe,
            Recipe::Conv1dAutoencoder { .. } | Recipe::LstmAutoencoder { .. }
        ),
        TaskKind::SequenceForecast => {
            sensor && regression && spec.prediction_horizon_ms.is_some_and(|h| h > 0)
        }
        TaskKind::VisualSequenceClassification => matches!(
            config.recipe,
            Recipe::CnnLstm {
                objective: Objective::Classification,
                ..
            }
        ),
        TaskKind::Fusion => {
            matches!(config.recipe, Recipe::ImageSensorFusion { .. })
                && if spec.labels.is_empty() {
                    regression
                } else {
                    classification
                }
        }
    };
    if !compatible {
        return Err(Error::Invalid(format!(
            "Recipe is incompatible with inspection task {:?}",
            spec.task
        )));
    }
    if (classification || config.recipe.is_detection() || config.recipe.is_segmentation())
        && config.recipe.outputs() != spec.labels.len()
    {
        return Err(Error::Invalid(
            "Model outputs must match the inspection label order".into(),
        ));
    }
    let mut shape = vec![1];
    shape.extend(&spec.input_shape);
    config.recipe.validate_shape(&shape)?;
    Ok(())
}

impl TrainingConfig {
    /// Tighter limits for an LLM-generated initial plan. Explicit user configurations use validate().
    pub fn validate_planning_budget(&self) -> Result<()> {
        self.validate()?;
        let bounded = match &self.recipe {
            Recipe::Mlp { hidden, .. }
            | Recipe::Lstm { hidden, .. }
            | Recipe::Gru { hidden, .. }
            | Recipe::Cnn1d { hidden, .. }
            | Recipe::Tcn { hidden, .. } => *hidden <= 128,
            Recipe::DenseAutoencoder { hidden, latent, .. }
            | Recipe::Conv1dAutoencoder { hidden, latent, .. }
            | Recipe::LstmAutoencoder { hidden, latent, .. } => *hidden <= 128 && *latent <= 128,
            Recipe::CnnLstm {
                hidden,
                cnn_channels,
                ..
            }
            | Recipe::ImageSensorFusion {
                hidden,
                cnn_channels,
                ..
            } => *hidden <= 128 && *cnn_channels <= 128,
            Recipe::ResNet18 { base_channels, .. } | Recipe::UNet { base_channels, .. } => {
                *base_channels <= 128
            }
            Recipe::MobileNetV2 {
                width_multiplier, ..
            }
            | Recipe::EfficientNet {
                width_multiplier, ..
            } => *width_multiplier <= 1.0,
            Recipe::YoloX {
                width_multiplier,
                depth_multiplier,
                ..
            } => *width_multiplier <= 1.0 && *depth_multiplier <= 1.0,
            Recipe::MaskRcnn { config } => config.base_channels <= 128,
        };
        if self.epochs > 100 || self.batch_size > 128 || !bounded {
            return Err(Error::Invalid(
                "Generated plan exceeds epoch, batch or model-width limits".into(),
            ));
        }
        Ok(())
    }
}

fn linear(input: u128, output: u128) -> u128 {
    (input + 1) * output
}
fn conv(input: u128, output: u128, kernel: u128) -> u128 {
    (input * kernel * kernel + 1) * output
}
fn conv1(input: u128, output: u128) -> u128 {
    (input * 3 + 1) * output
}
fn recurrent(input: u128, hidden: u128, gates: u128) -> u128 {
    gates * hidden * (input + hidden + 2)
}
fn image_encoder(input: u128, width: u128) -> u128 {
    conv(input, width, 3) + conv(width, width * 2, 3) + conv(width * 2, width * 4, 3)
}
fn base_conv(input: u128, output: u128, kernel: u128) -> u128 {
    input * output * kernel * kernel + 4 * output
}
fn csp(input: u128, output: u128, depth: u128) -> u128 {
    let hidden = output / 2;
    base_conv(input, hidden, 1) * 2
        + depth * (base_conv(hidden, hidden, 1) + base_conv(hidden, hidden, 3))
        + base_conv(hidden * 2, output, 1)
}
impl crate::EfficientAdConfig {
    /// Includes both student branches, the supplied teacher and the configured autoencoder width.
    pub fn estimated_training_bytes(&self) -> Result<u64> {
        self.validate()?;
        let teacher = self
            .teacher
            .convolutions
            .iter()
            .map(|layer| (layer.weight.values.len() + layer.bias.values.len()) as u128)
            .sum::<u128>();
        let last = &self.teacher.convolutions[3];
        let mut parameters =
            teacher * 2 + (last.weight.values.len() + last.bias.values.len()) as u128;
        let width = self.autoencoder_channels as u128;
        let input = self.teacher.input_channels as u128;
        let features = self.teacher.feature_channels as u128;
        let channels = [input, width / 2, width / 2, width, width, width, width];
        for i in 0..6 {
            parameters += conv(channels[i], channels[i + 1], if i == 5 { 8 } else { 4 });
        }
        parameters += 6 * conv(width, width, 4) + conv(width, width, 3) + conv(width, features, 3);
        let pdn_width = self.teacher.base_channels as u128;
        let pdn = (253 * 253 + 126 * 126) * pdn_width
            + (123 * 123 + 61 * 61 + 59 * 59) * pdn_width * 2
            + 56 * 56 * features * 2;
        let ae = (128 * 128 + 64 * 64) * width / 2
            + (32 * 32 + 16 * 16 + 8 * 8 + 1) * width
            + [3u128, 4, 8, 9, 15, 16, 32, 33, 63, 64, 127, 128, 56, 56]
                .iter()
                .map(|size| size * size * width)
                .sum::<u128>()
            + 56 * 56 * features;
        let total = parameters * 32
            + (pdn * 3 + ae) * self.batch_size as u128 * 128
            + input * 256 * 256 * self.batch_size as u128 * 16
            + 16 * 1024 * 1024;
        u64::try_from(total)
            .map_err(|_| Error::Invalid("EfficientAD memory estimate overflows".into()))
    }
}
impl Recipe {
    /// Conservative live training allocation estimate, including parameters, Adam state,
    /// gradient/update copies, saved activations and convolution workspaces. Dataset storage is extra.
    pub fn estimated_training_bytes(
        &self,
        sample_shape: &[usize],
        batch_size: usize,
    ) -> Result<u64> {
        if batch_size == 0 || batch_size > 4096 {
            return Err(Error::Invalid(
                "Training batch size must be 1..=4096".into(),
            ));
        }
        let mut shape = vec![batch_size];
        shape.extend(sample_shape);
        self.validate_shape(&shape)?;
        let b = batch_size as u128;
        let elements = sample_shape.iter().map(|v| *v as u128).product::<u128>();
        let time = sample_shape.first().copied().unwrap_or(1) as u128;
        let image_area = if sample_shape.len() >= 3 {
            (sample_shape[sample_shape.len() - 2] as u128)
                * (sample_shape[sample_shape.len() - 1] as u128)
        } else {
            1
        };
        let area_at = |stride: u128| -> u128 {
            if sample_shape.len() < 3 {
                1
            } else {
                (sample_shape[sample_shape.len() - 2] as u128).div_ceil(stride)
                    * (sample_shape[sample_shape.len() - 1] as u128).div_ceil(stride)
            }
        };
        let (parameters, activations) = match self {
            Self::Mlp {
                input_features,
                hidden,
                outputs,
                ..
            } => {
                let (i, h, o) = (*input_features as u128, *hidden as u128, *outputs as u128);
                (linear(i, h) + linear(h, o), b * (i + h + o) * 64)
            }
            Self::Lstm {
                input_features,
                hidden,
                outputs,
                ..
            }
            | Self::Gru {
                input_features,
                hidden,
                outputs,
                ..
            } => {
                let (i, h, o) = (*input_features as u128, *hidden as u128, *outputs as u128);
                let gates = if matches!(self, Self::Lstm { .. }) {
                    4
                } else {
                    3
                };
                (
                    recurrent(i, h, gates) + linear(h, o),
                    b * time * (i + h * 16 + o) * 64,
                )
            }
            Self::Cnn1d {
                input_features,
                hidden,
                outputs,
                ..
            } => {
                let (i, h, o) = (*input_features as u128, *hidden as u128, *outputs as u128);
                (
                    conv1(i, h) + conv1(h, h) + linear(h, o),
                    b * time * (i + h * 6 + o) * 128,
                )
            }
            Self::Tcn {
                input_features,
                hidden,
                levels,
                outputs,
                ..
            } => {
                let (i, h, l, o) = (
                    *input_features as u128,
                    *hidden as u128,
                    *levels as u128,
                    *outputs as u128,
                );
                (
                    conv1(i, h)
                        + conv1(h, h)
                        + (l - 1) * conv1(h, h) * 2
                        + if i != h { linear(i, h) } else { 0 }
                        + linear(h, o),
                    b * time * (i + h * l * 6 + o) * 128,
                )
            }
            Self::DenseAutoencoder {
                input_features,
                hidden,
                latent,
            } => {
                let (i, h, l) = (*input_features as u128, *hidden as u128, *latent as u128);
                (
                    linear(i, h) + linear(h, l) + linear(l, h) + linear(h, i),
                    b * (i * 2 + h * 2 + l) * 64,
                )
            }
            Self::Conv1dAutoencoder {
                input_features,
                hidden,
                latent,
            } => {
                let (i, h, l) = (*input_features as u128, *hidden as u128, *latent as u128);
                (
                    conv1(i, h) + conv1(h, l) + conv1(l, h) + conv1(h, i),
                    b * time * (i * 2 + h * 2 + l) * 128,
                )
            }
            Self::LstmAutoencoder {
                input_features,
                hidden,
                latent,
            } => {
                let (i, h, l) = (*input_features as u128, *hidden as u128, *latent as u128);
                (
                    recurrent(i, h, 4) + linear(h, l) + recurrent(l, h, 4) + linear(h, i),
                    b * time * (i * 2 + h * 32 + l) * 64,
                )
            }
            Self::CnnLstm {
                input_channels,
                cnn_channels,
                hidden,
                outputs,
                ..
            } => {
                let (i, c, h, o) = (
                    *input_channels as u128,
                    *cnn_channels as u128,
                    *hidden as u128,
                    *outputs as u128,
                );
                (
                    image_encoder(i, c) + recurrent(c * 4, h, 4) + linear(h, o),
                    b * time * (image_area * (i + c * 6) * 128 + h * 1024),
                )
            }
            Self::ImageSensorFusion {
                input_channels,
                height,
                width,
                sensor_features,
                cnn_channels,
                hidden,
                outputs,
                ..
            } => {
                let (i, c, h, o) = (
                    *input_channels as u128,
                    *cnn_channels as u128,
                    *hidden as u128,
                    *outputs as u128,
                );
                (
                    image_encoder(i, c)
                        + linear(*sensor_features as u128, h)
                        + linear(c * 4 + h, h)
                        + linear(h, o),
                    b * (*height as u128) * (*width as u128) * (i + c * 6) * 128 + b * h * 256,
                )
            }
            Self::ResNet18 {
                input_channels,
                classes,
                base_channels,
            } => {
                let w = *base_channels as u128;
                let mut p = base_conv(*input_channels as u128, w, 7);
                let mut previous = w;
                for level in 0..4 {
                    let c = w << level;
                    for block in 0..2 {
                        p += base_conv(previous, c, 3) + base_conv(c, c, 3);
                        if previous != c || (level > 0 && block == 0) {
                            p += base_conv(previous, c, 1);
                        }
                        previous = c;
                    }
                }
                (
                    p + linear(w * 8, *classes as u128),
                    b * (w * area_at(2)
                        + w * area_at(4)
                        + (0..4)
                            .map(|level| {
                                (w << level) * area_at(4 << level) * if level == 0 { 4 } else { 5 }
                            })
                            .sum::<u128>()
                        + image_area * (*input_channels as u128))
                        * 128,
                )
            }
            Self::UNet {
                input_channels,
                classes,
                base_channels,
                depth,
            } => {
                let w = *base_channels as u128;
                let mut previous = *input_channels as u128;
                let mut p = 0;
                let mut maps = 0;
                for level in 0..=*depth {
                    let c = w << level;
                    p += conv(previous, c, 3) + conv(c, c, 3);
                    maps += c * image_area / (1u128 << (level * 2));
                    previous = c;
                }
                for level in (0..*depth).rev() {
                    let c = w << level;
                    p += conv(previous + c, c, 3) + conv(c, c, 3);
                    maps += c * image_area / (1u128 << (level * 2)) * 2;
                    previous = c;
                }
                (
                    p + conv(w, *classes as u128, 1),
                    b * (maps * 2 + image_area * (*input_channels + *classes) as u128) * 128,
                )
            }
            Self::MobileNetV2 {
                input_channels,
                classes,
                width_multiplier,
            }
            | Self::EfficientNet {
                input_channels,
                classes,
                width_multiplier,
            } => {
                // Rounded channel widths and squeeze/excitation layers are covered by this bound.
                let scale = width_multiplier.max(1.0);
                let channels = (1280.0 * scale).ceil() as u128 + 8;
                let multiplier = (scale * scale).ceil() as u128;
                (
                    12_000_000 * multiplier
                        + conv(*input_channels as u128, 64, 3)
                        + linear(channels, *classes as u128),
                    b * image_area
                        * (*input_channels as u128 + (256.0 * scale).ceil() as u128)
                        * 64,
                )
            }
            Self::YoloX {
                input_channels,
                classes,
                width_multiplier,
                depth_multiplier,
            } => {
                let w = ((64.0 * width_multiplier) as usize / 2 * 2).max(4) as u128;
                let n = (3.0 * depth_multiplier).round().max(1.0) as u128;
                let (c3, c4, c5) = (w * 4, w * 8, w * 16);
                let mut p = base_conv(*input_channels as u128 * 4, w, 3);
                for level in 0..4 {
                    let c = w << (level + 1);
                    p += base_conv(w << level, c, 3)
                        + csp(c, c, if level == 1 || level == 2 { n * 3 } else { n });
                }
                p += base_conv(c5, c5 / 2, 1)
                    + base_conv(c5 * 2, c5, 1)
                    + base_conv(c5, c4, 1)
                    + csp(c4 * 2, c4, n)
                    + base_conv(c4, c3, 1)
                    + csp(c3 * 2, c3, n)
                    + base_conv(c3, c3, 3)
                    + csp(c3 * 2, c4, n)
                    + base_conv(c4, c4, 3)
                    + csp(c4 * 2, c5, n);
                for input in [c3, c4, c5] {
                    p += base_conv(input, c3, 1)
                        + 4 * base_conv(c3, c3, 3)
                        + conv(c3, 5 + *classes as u128, 1);
                }
                {
                    let mut maps = w * area_at(2);
                    for level in 0..4 {
                        let c = w << (level + 1);
                        let depth = if level == 1 || level == 2 { n * 3 } else { n };
                        maps += c * (3 + depth) * area_at(4 << level);
                    }
                    maps += c5 * 5 * area_at(32)
                        + c4 * area_at(32)
                        + c4 * (5 + n) * area_at(16)
                        + c3 * area_at(16)
                        + c3 * (5 + n) * area_at(8)
                        + (c3 * 3 + c4 * (2 + n)) * area_at(16)
                        + (c4 * 3 + c5 * (2 + n)) * area_at(32);
                    maps +=
                        (5 * c3 + 5 + *classes as u128) * (area_at(8) + area_at(16) + area_at(32));
                    (p, b * (maps + image_area * (*input_channels as u128)) * 128)
                }
            }
            Self::MaskRcnn { config } => {
                let c = config.base_channels as u128;
                let a = (config.anchor_scales.len() * config.anchor_ratios.len()) as u128;
                let hidden = c * 4;
                let classes = config.classes as u128;
                let mut p = 0;
                for (i, o) in [
                    (config.input_channels as u128, c),
                    (c, c),
                    (c, c * 2),
                    (c * 2, c * 2),
                ] {
                    p += conv(i, o, 3) + conv(o, o, 3) + conv(i, o, 1);
                }
                p += conv(c * 2, c * 2, 3)
                    + conv(c * 2, a * 5, 1)
                    + linear(c * 2 * (config.roi_size * config.roi_size) as u128, hidden)
                    + linear(hidden, hidden)
                    + linear(hidden, classes + 1)
                    + linear(hidden, classes * 4)
                    + 4 * conv(c * 2, c * 2, 3)
                    + conv(c * 2, c * 2, 2)
                    + conv(c * 2, classes, 1);
                let rois = config.training_samples.max(config.proposals) as u128;
                (
                    p,
                    b * (image_area * config.input_channels as u128
                        + area_at(2) * c * 4
                        + area_at(4) * (c * 10 + a * 8))
                        * 128
                        + b * rois
                            * (config.mask_size * config.mask_size) as u128
                            * (c * 16 + classes * 4)
                            * 128,
                )
            }
        };
        let bytes = parameters * 32 + activations + b * elements * 16 + 16 * 1024 * 1024;
        u64::try_from(bytes)
            .map_err(|_| Error::Invalid("Training memory estimate overflows".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(recipe: Recipe) -> TrainingConfig {
        TrainingConfig {
            recipe,
            backend: Default::default(),
            epochs: 10,
            batch_size: 4,
            learning_rate: 0.001,
            seed: 7,
            gradient_clip: 5.0,
        }
    }
    fn spec(task: TaskKind, shape: Vec<usize>, labels: Vec<String>) -> InspectionSpec {
        InspectionSpec {
            id: "inspection".into(),
            description: "Fixture".into(),
            task,
            input_shape: shape,
            labels,
            prediction_horizon_ms: None,
            minimum_examples: 1,
            minimum_examples_per_class: 1,
        }
    }
    #[test]
    fn tasks_cannot_silently_change_training_objectives() {
        let labels = vec!["normal".into(), "defect".into()];
        let settings = config(Recipe::Mlp {
            input_features: 2,
            hidden: 4,
            outputs: 2,
            objective: Objective::Regression,
        });
        assert!(
            validate_inspection_config(
                &spec(TaskKind::SensorClassification, vec![2], labels.clone()),
                &settings
            )
            .is_err()
        );
        let settings = config(Recipe::UNet {
            input_channels: 3,
            classes: 2,
            base_channels: 4,
            depth: 1,
        });
        assert!(
            validate_inspection_config(
                &spec(
                    TaskKind::ImageClassification,
                    vec![3, 32, 32],
                    labels.clone()
                ),
                &settings
            )
            .is_err()
        );
        assert!(
            validate_inspection_config(
                &spec(TaskKind::Segmentation, vec![3, 32, 32], labels),
                &settings
            )
            .is_ok()
        );
        let settings = config(Recipe::Lstm {
            input_features: 1,
            hidden: 4,
            outputs: 3,
            objective: Objective::Regression,
        });
        let mut target = spec(TaskKind::SequenceForecast, vec![4, 1], vec![]);
        assert!(validate_inspection_config(&target, &settings).is_err());
        target.prediction_horizon_ms = Some(100);
        assert!(validate_inspection_config(&target, &settings).is_ok());
        let fusion = config(Recipe::ImageSensorFusion {
            input_channels: 1,
            height: 4,
            width: 4,
            sensor_features: 1,
            cnn_channels: 2,
            hidden: 4,
            outputs: 1,
            objective: Objective::Regression,
        });
        assert!(
            validate_inspection_config(&spec(TaskKind::Fusion, vec![17], vec![]), &fusion).is_ok()
        );
        assert!(
            validate_inspection_config(
                &spec(TaskKind::Fusion, vec![17], vec!["defect".into()]),
                &fusion
            )
            .is_err()
        );
        let detector = config(Recipe::YoloX {
            input_channels: 3,
            classes: 1,
            width_multiplier: 0.25,
            depth_multiplier: 0.33,
        });
        assert!(
            validate_inspection_config(
                &spec(
                    TaskKind::ObjectDetection,
                    vec![3, 32, 32],
                    vec!["defect".into()]
                ),
                &detector
            )
            .is_ok()
        );
        assert!(
            validate_inspection_config(
                &spec(
                    TaskKind::ObjectDetection,
                    vec![3, 32, 32],
                    vec!["one".into(), "two".into()]
                ),
                &detector
            )
            .is_err()
        );
    }
    #[test]
    fn memory_estimates_cover_large_temporal_models_and_grow_with_batches() {
        let recipe = Recipe::Tcn {
            input_features: 1,
            hidden: 4096,
            levels: 12,
            outputs: 2,
            objective: Objective::Classification,
        };
        assert!(recipe.estimated_training_bytes(&[4, 1], 1).unwrap() > 10_000_000_000);
        let recipe = Recipe::Cnn1d {
            input_features: 65536,
            hidden: 4096,
            outputs: 2,
            objective: Objective::Classification,
        };
        assert!(recipe.estimated_training_bytes(&[1, 65536], 1).unwrap() > 20_000_000_000);
        let recipe = Recipe::ResNet18 {
            input_channels: 3,
            classes: 2,
            base_channels: 4,
        };
        assert!(
            recipe.estimated_training_bytes(&[3, 32, 32], 8).unwrap()
                > recipe.estimated_training_bytes(&[3, 32, 32], 1).unwrap()
        );
        assert!(
            recipe
                .estimated_training_bytes(&[3, usize::MAX, 32], 1)
                .is_err()
        );
        assert!(
            config(Recipe::Lstm {
                input_features: 1,
                hidden: 256,
                outputs: 2,
                objective: Objective::Classification
            })
            .validate_planning_budget()
            .is_err()
        );
    }
}

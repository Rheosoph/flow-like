use crate::{BackendChoice, Error, Result, TensorData};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct ConvWeights {
    pub weight: TensorData,
    pub bias: TensorData,
}

/// PDN-small convolution weights in OIHW order, including all four bias vectors.
/// Canonical pretrained teachers use base_channels=128 and feature_channels=384.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct PdnTeacherWeights {
    pub input_channels: usize,
    pub base_channels: usize,
    pub feature_channels: usize,
    pub convolutions: Vec<ConvWeights>,
}
impl PdnTeacherWeights {
    pub fn validate(&self) -> Result<()> {
        if self.input_channels == 0
            || self.input_channels > 16
            || self.base_channels == 0
            || self.base_channels > 256
            || self.feature_channels == 0
            || self.feature_channels > 1024
            || self.convolutions.len() != 4
        {
            return Err(Error::Invalid(
                "PDN-small teacher dimensions are invalid".into(),
            ));
        }
        let width = self.base_channels;
        let shapes = [
            [width, self.input_channels, 4, 4],
            [width * 2, width, 4, 4],
            [width * 2, width * 2, 3, 3],
            [self.feature_channels, width * 2, 4, 4],
        ];
        let mut count = 0usize;
        for (weights, shape) in self.convolutions.iter().zip(shapes) {
            weights.weight.validate()?;
            weights.bias.validate()?;
            if weights.weight.shape != shape || weights.bias.shape != [shape[0]] {
                return Err(Error::Invalid(
                    "Teacher weights do not match PDN-small OIHW dimensions".into(),
                ));
            }
            count += weights.weight.values.len() + weights.bias.values.len();
        }
        if count > 67_108_864 {
            return Err(Error::Invalid(
                "Teacher exceeds the 256 MiB float32 weight limit".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct EfficientAdConfig {
    pub teacher: PdnTeacherWeights,
    #[serde(default)]
    pub backend: BackendChoice,
    pub epochs: usize,
    pub batch_size: usize,
    pub learning_rate: f64,
    pub seed: u64,
    /// Canonical autoencoder width is 64. Its first two layers use half this width.
    pub autoencoder_channels: usize,
    /// Quantile of teacher/student squared distances retained by the hard-example loss.
    pub hard_quantile: f32,
    pub gradient_clip: f32,
}
impl EfficientAdConfig {
    pub fn validate(&self) -> Result<()> {
        self.teacher.validate()?;
        if self.epochs == 0
            || self.batch_size == 0
            || self.batch_size > 4096
            || !self.learning_rate.is_finite()
            || self.learning_rate <= 0.0
            || self.learning_rate > 1.0
            || self.autoencoder_channels < 2
            || self.autoencoder_channels > 256
            || !self.hard_quantile.is_finite()
            || !(0.0..1.0).contains(&self.hard_quantile)
            || !self.gradient_clip.is_finite()
            || self.gradient_clip <= 0.0
        {
            return Err(Error::Invalid(
                "EfficientAD training settings are invalid".into(),
            ));
        }
        Ok(())
    }
}

/// All images are already normalized float32 NCHW tensors at 256 by 256 pixels.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct EfficientAdDataset {
    pub images: TensorData,
    /// Optional photometric augmentations of the same training samples for the autoencoder branch.
    pub autoencoder_images: Option<TensorData>,
    /// Optional out-of-distribution images for the student pretraining penalty.
    pub penalty_images: Option<TensorData>,
}
impl EfficientAdDataset {
    pub fn validate(&self, channels: usize) -> Result<()> {
        validate_images(&self.images, channels)?;
        if let Some(images) = &self.autoencoder_images {
            validate_images(images, channels)?;
            if images.shape != self.images.shape {
                return Err(Error::Invalid(
                    "Autoencoder augmentations must match the training batch".into(),
                ));
            }
        }
        if let Some(images) = &self.penalty_images {
            validate_images(images, channels)?;
        }
        Ok(())
    }
    #[cfg(feature = "engine")]
    pub(crate) fn batch(&self, indices: &[usize]) -> Self {
        Self {
            images: self.images.batch(indices),
            autoencoder_images: self.autoencoder_images.as_ref().map(|x| x.batch(indices)),
            penalty_images: self
                .penalty_images
                .as_ref()
                .map(|x| x.batch(&indices.iter().map(|i| i % x.shape[0]).collect::<Vec<_>>())),
        }
    }
}
pub(crate) fn validate_images(images: &TensorData, channels: usize) -> Result<()> {
    images.validate()?;
    if images.shape.len() != 4 || images.shape[1..] != [channels, 256, 256] {
        return Err(Error::Invalid(
            "EfficientAD requires normalized NCHW images of size 256x256".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct EfficientAdCalibration {
    pub teacher_student_low: f32,
    pub teacher_student_high: f32,
    pub autoencoder_low: f32,
    pub autoencoder_high: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EfficientAdPrediction {
    pub anomaly_maps: TensorData,
    pub teacher_student_maps: TensorData,
    pub autoencoder_maps: TensorData,
    pub scores: Vec<f32>,
}

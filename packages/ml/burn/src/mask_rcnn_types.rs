use crate::{BoundingBox, Detection, Error, Result, TensorData};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct MaskRcnnConfig {
    pub input_channels: usize,
    pub classes: usize,
    pub base_channels: usize,
    pub anchor_scales: Vec<f32>,
    pub anchor_ratios: Vec<f32>,
    pub roi_size: usize,
    pub mask_size: usize,
    pub proposals: usize,
    pub training_samples: usize,
}
impl Default for MaskRcnnConfig {
    fn default() -> Self {
        Self {
            input_channels: 3,
            classes: 2,
            base_channels: 32,
            anchor_scales: vec![0.125, 0.25, 0.5],
            anchor_ratios: vec![0.5, 1.0, 2.0],
            roi_size: 7,
            mask_size: 28,
            proposals: 128,
            training_samples: 64,
        }
    }
}
impl MaskRcnnConfig {
    pub fn validate(&self) -> Result<()> {
        if !(1..=16).contains(&self.input_channels)
            || !(1..=256).contains(&self.classes)
            || !(2..=256).contains(&self.base_channels)
            || !(1..=14).contains(&self.roi_size)
            || !(2..=56).contains(&self.mask_size)
            || !self.mask_size.is_multiple_of(2)
            || !(1..=512).contains(&self.proposals)
            || !(4..=512).contains(&self.training_samples)
            || self.anchor_scales.is_empty()
            || self.anchor_scales.len() > 8
            || self.anchor_ratios.is_empty()
            || self.anchor_ratios.len() > 8
            || self
                .anchor_scales
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v > 2.0)
            || self
                .anchor_ratios
                .iter()
                .any(|v| !v.is_finite() || *v < 0.1 || *v > 10.0)
        {
            return Err(Error::Invalid(
                "Invalid Mask R-CNN dimensions, anchors or proposal limits".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct InstanceTarget {
    pub bbox: BoundingBox,
    /// Full-image binary mask, shaped [height, width].
    pub mask: TensorData,
}
impl InstanceTarget {
    pub fn validate(&self, classes: usize, height: usize, width: usize) -> Result<()> {
        let b = &self.bbox;
        self.mask.validate()?;
        if b.class_id >= classes
            || [b.x_min, b.y_min, b.x_max, b.y_max]
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || b.x_max <= b.x_min
            || b.y_max <= b.y_min
            || self.mask.shape != [height, width]
            || self.mask.values.iter().any(|v| *v != 0.0 && *v != 1.0)
        {
            return Err(Error::Invalid(
                "Instance target needs a valid normalized box and a full-image binary mask".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct InstanceDetection {
    pub detection: Detection,
    pub mask: TensorData,
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct MaskRcnnPrediction {
    pub instances: Vec<Vec<InstanceDetection>>,
}

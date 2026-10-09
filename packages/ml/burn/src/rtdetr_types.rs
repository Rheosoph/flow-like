use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq,
)]
#[serde(rename_all = "snake_case")]
pub enum RtdetrV2Backbone {
    #[default]
    ResNet18,
    ResNet50,
}

/// RT-DETRv2 with the official ResNet-vd backbone and three-level hybrid encoder.
/// Fine-tuning uses matching and auxiliary losses without denoising query augmentation.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct RtdetrV2Config {
    pub backbone: RtdetrV2Backbone,
    pub classes: usize,
    #[serde(default = "default_queries")]
    pub queries: usize,
}
fn default_queries() -> usize {
    300
}
impl Default for RtdetrV2Config {
    fn default() -> Self {
        Self {
            backbone: RtdetrV2Backbone::ResNet18,
            classes: 80,
            queries: default_queries(),
        }
    }
}
impl RtdetrV2Config {
    pub fn validate(&self) -> Result<()> {
        if !(1..=4096).contains(&self.classes) || !(1..=1000).contains(&self.queries) {
            return Err(Error::Invalid(
                "RT-DETRv2 requires 1 to 4096 classes and 1 to 1000 object queries".into(),
            ));
        }
        Ok(())
    }
    pub fn validate_input_shape(&self, shape: &[usize]) -> Result<()> {
        self.validate()?;
        let [batch, channels, height, width] = shape else {
            return Err(Error::Invalid(
                "RT-DETRv2 input must have shape [N, 3, H, W]".into(),
            ));
        };
        if *batch == 0
            || *channels != 3
            || *height < 32
            || *width < 32
            || height % 32 != 0
            || width % 32 != 0
            || *height > 1536
            || *width > 1536
        {
            return Err(Error::Invalid("RT-DETRv2 needs RGB images whose dimensions are multiples of 32 between 32 and 1536".into()));
        }
        let locations =
            height / 8 * (width / 8) + height / 16 * (width / 16) + height / 32 * (width / 32);
        if self.queries > locations {
            return Err(Error::Invalid(
                "RT-DETRv2 object queries exceed the input's feature locations".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn layers(&self) -> usize {
        match self.backbone {
            RtdetrV2Backbone::ResNet18 => 3,
            RtdetrV2Backbone::ResNet50 => 6,
        }
    }
}

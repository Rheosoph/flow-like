use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Apache-2.0 RF-DETR detection variants with the windowed ViT-S backbone.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq,
)]
#[serde(rename_all = "snake_case")]
pub enum RfDetrVariant {
    Base,
    #[default]
    Nano,
    Small,
    Medium,
    Large2026,
}
impl RfDetrVariant {
    pub const fn resolution(self) -> usize {
        match self {
            Self::Base => 560,
            Self::Nano => 384,
            Self::Small => 512,
            Self::Medium => 576,
            Self::Large2026 => 704,
        }
    }
    pub const fn patch_size(self) -> usize {
        if matches!(self, Self::Base) { 14 } else { 16 }
    }
    pub const fn positional_grid(self) -> usize {
        match self {
            Self::Base => 37,
            Self::Nano => 24,
            Self::Small => 32,
            Self::Medium => 36,
            Self::Large2026 => 44,
        }
    }
    pub const fn windows(self) -> usize {
        if matches!(self, Self::Base) { 4 } else { 2 }
    }
    pub const fn decoder_layers(self) -> usize {
        match self {
            Self::Base | Self::Small => 3,
            Self::Nano => 2,
            Self::Medium | Self::Large2026 => 4,
        }
    }
    pub const fn feature_stages(self) -> [usize; 4] {
        if matches!(self, Self::Base) {
            [2, 5, 8, 11]
        } else {
            [3, 6, 9, 12]
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct RfDetrConfig {
    pub variant: RfDetrVariant,
    /// Number of sigmoid score channels. Official COCO checkpoints contain 91 channels.
    pub classes: usize,
}
impl Default for RfDetrConfig {
    fn default() -> Self {
        Self {
            variant: RfDetrVariant::Nano,
            classes: 2,
        }
    }
}
impl RfDetrConfig {
    pub const QUERIES: usize = 300;
    pub const TRAINING_GROUPS: usize = 13;
    pub fn validate(&self) -> Result<()> {
        if !(1..=1024).contains(&self.classes) {
            return Err(Error::Invalid(
                "RF-DETR requires between 1 and 1024 score channels".into(),
            ));
        }
        Ok(())
    }
    pub fn validate_input_shape(&self, shape: &[usize]) -> Result<()> {
        self.validate()?;
        let [batch, channels, height, width] = shape else {
            return Err(Error::Invalid(
                "RF-DETR input must have shape [N, 3, H, W]".into(),
            ));
        };
        let patch = self.variant.patch_size();
        let divisor = patch * self.variant.windows();
        let patches = (height / patch).checked_mul(width / patch).unwrap_or(0);
        if *batch == 0
            || *channels != 3
            || !height.is_multiple_of(divisor)
            || !width.is_multiple_of(divisor)
            || !(Self::QUERIES..=4096).contains(&patches)
        {
            return Err(Error::Invalid(format!(
                "RF-DETR requires RGB images with dimensions divisible by {divisor} and between 300 and 4096 patches"
            )));
        }
        Ok(())
    }
}

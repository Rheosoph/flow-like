use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Official DINOv2 backbones without register tokens, using 14 × 14 RGB patches.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq,
)]
#[serde(rename_all = "snake_case")]
pub enum DinoV2Variant {
    #[default]
    Small,
    Base,
}
impl DinoV2Variant {
    pub const fn embedding_dim(self) -> usize {
        match self {
            Self::Small => 384,
            Self::Base => 768,
        }
    }
    pub const fn heads(self) -> usize {
        match self {
            Self::Small => 6,
            Self::Base => 12,
        }
    }
    pub const fn depth(self) -> usize {
        12
    }
}

/// A DINOv2 backbone with a supervised linear classifier on the normalized CLS token.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct DinoV2Config {
    pub variant: DinoV2Variant,
    pub classes: usize,
}
impl Default for DinoV2Config {
    fn default() -> Self {
        Self {
            variant: DinoV2Variant::Small,
            classes: 2,
        }
    }
}
impl DinoV2Config {
    pub const PATCH_SIZE: usize = 14;
    pub const PRETRAINED_GRID: usize = 37;

    pub fn validate(&self) -> Result<()> {
        if !(1..=65_536).contains(&self.classes) {
            return Err(Error::Invalid(
                "DINOv2 requires between 1 and 65536 classifier outputs".into(),
            ));
        }
        Ok(())
    }

    pub fn validate_input_shape(&self, shape: &[usize]) -> Result<()> {
        self.validate()?;
        let [batch, channels, height, width] = shape else {
            return Err(Error::Invalid(
                "DINOv2 input must have shape [N, 3, H, W]".into(),
            ));
        };
        if *batch == 0
            || *channels != 3
            || *height == 0
            || *width == 0
            || !height.is_multiple_of(Self::PATCH_SIZE)
            || !width.is_multiple_of(Self::PATCH_SIZE)
            || (height / Self::PATCH_SIZE)
                .checked_mul(width / Self::PATCH_SIZE)
                .is_none_or(|patches| patches > 4096)
        {
            return Err(Error::Invalid(
                "DINOv2 needs nonempty RGB images with dimensions divisible by 14 and at most 4096 patches per image".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variants_keep_official_backbone_dimensions() {
        assert_eq!(
            (
                DinoV2Variant::Small.embedding_dim(),
                DinoV2Variant::Small.heads(),
                DinoV2Variant::Small.depth()
            ),
            (384, 6, 12)
        );
        assert_eq!(
            (
                DinoV2Variant::Base.embedding_dim(),
                DinoV2Variant::Base.heads(),
                DinoV2Variant::Base.depth()
            ),
            (768, 12, 12)
        );
        let config = DinoV2Config::default();
        assert!(config.validate_input_shape(&[2, 3, 518, 518]).is_ok());
        assert!(config.validate_input_shape(&[1, 3, 224, 336]).is_ok());
        for shape in [
            [1, 3, 225, 224],
            [1, 1, 224, 224],
            [0, 3, 14, 14],
            [1, 3, 910, 910],
        ] {
            assert!(config.validate_input_shape(&shape).is_err());
        }
    }
}

use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// D-FINE Nano with HGNetv2-B0, distribution refinement and location quality estimation.
/// Fine-tuning uses matched classification/box losses and auxiliary predictions.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct DfineConfig {
    pub classes: usize,
    #[serde(default = "default_queries")]
    pub queries: usize,
}

fn default_queries() -> usize {
    300
}

impl Default for DfineConfig {
    fn default() -> Self {
        Self {
            classes: 80,
            queries: default_queries(),
        }
    }
}

impl DfineConfig {
    pub fn validate(&self) -> Result<()> {
        if !(1..=4096).contains(&self.classes) || !(1..=1000).contains(&self.queries) {
            return Err(Error::Invalid(
                "D-FINE requires 1 to 4096 classes and 1 to 1000 queries".into(),
            ));
        }
        Ok(())
    }

    pub fn validate_input_shape(&self, shape: &[usize]) -> Result<()> {
        self.validate()?;
        let [batch, channels, height, width] = shape else {
            return Err(Error::Invalid(
                "D-FINE input must have shape [N,3,H,W]".into(),
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
            || self.queries > height / 16 * (width / 16) + height / 32 * (width / 32)
        {
            return Err(Error::Invalid("D-FINE needs RGB images with dimensions divisible by 32 up to 1536, and enough feature locations for its queries".into()));
        }
        Ok(())
    }
}

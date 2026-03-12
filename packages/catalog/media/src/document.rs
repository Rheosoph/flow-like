use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod chart;
pub mod docx;
pub mod pdf;
pub mod pptx;
pub mod styles;

#[cfg(feature = "execute")]
pub mod openxml;

/// Controls how replaced images are scaled relative to the original placeholder.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub enum ImageScaleMode {
    /// Don't preserve original dimensions — use the new image's native size
    None,
    /// Keep the original width, scale height proportionally
    #[default]
    KeepWidth,
    /// Keep the original height, scale width proportionally
    KeepHeight,
    /// Force both original width and height (may distort the image)
    Stretch,
}

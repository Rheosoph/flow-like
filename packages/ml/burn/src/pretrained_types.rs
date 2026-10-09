use crate::{Error, Recipe, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct FineTuneOptions {
    #[serde(default = "replace_head_by_default")]
    pub replace_head: bool,
    #[serde(default)]
    pub freeze_backbone: bool,
}
fn replace_head_by_default() -> bool {
    true
}
impl Default for FineTuneOptions {
    fn default() -> Self {
        Self {
            replace_head: true,
            freeze_backbone: false,
        }
    }
}

/// Check architecture compatibility before a new dataset starts fine-tuning.
pub fn validate_fine_tune(
    source: &Recipe,
    target: &Recipe,
    options: &FineTuneOptions,
) -> Result<()> {
    source.validate()?;
    target.validate()?;
    let mut expected = source.clone();
    let classifier = match &mut expected {
        Recipe::DfineNano { config } => {
            if options.replace_head {
                config.classes = target.outputs();
            }
            true
        }
        Recipe::RfDetr { config } => {
            if options.replace_head {
                config.classes = target.outputs();
            }
            true
        }
        Recipe::RtdetrV2 { config } => {
            if options.replace_head {
                config.classes = target.outputs();
            }
            true
        }
        Recipe::DinoV2 { config } => {
            if options.replace_head {
                config.classes = target.outputs();
            }
            true
        }
        Recipe::ResNet18 { classes, .. }
        | Recipe::MobileNetV2 { classes, .. }
        | Recipe::EfficientNet { classes, .. } => {
            if options.replace_head {
                *classes = target.outputs();
            }
            true
        }
        _ => false,
    };
    if !classifier && (options.replace_head || options.freeze_backbone) {
        return Err(Error::Invalid(
            "Head replacement and backbone freezing require a supported vision model".into(),
        ));
    }
    if expected != *target {
        return Err(Error::Invalid(
            "Pretrained and target recipes must use the same backbone; changing outputs requires head replacement".into(),
        ));
    }
    Ok(())
}

/// Caller-supplied origin information is retained alongside the verified weight digest.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct PretrainedWeightsMetadata {
    pub source: String,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub weights_license: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}
impl PretrainedWeightsMetadata {
    pub fn validate(&self) -> Result<()> {
        if self.source.trim().is_empty()
            || self.source.len() > 8192
            || [&self.license, &self.weights_license, &self.revision]
                .iter()
                .filter_map(|value| value.as_ref())
                .any(|value| value.trim().is_empty() || value.len() > 8192)
            || self.expected_sha256.as_ref().is_some_and(|digest| {
                digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(Error::Invalid(
                "Invalid pretrained weight provenance".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct ImportedWeightsProvenance {
    pub metadata: PretrainedWeightsMetadata,
    pub format: String,
    pub sha256: String,
    pub classification_head_pretrained: bool,
}
impl ImportedWeightsProvenance {
    pub fn validate(&self) -> Result<()> {
        self.metadata.validate()?;
        if self.format.trim().is_empty()
            || self.format.len() > 256
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self
                .metadata
                .expected_sha256
                .as_ref()
                .is_some_and(|expected| !expected.eq_ignore_ascii_case(&self.sha256))
        {
            return Err(Error::Invalid("Invalid imported weight provenance".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ImportedModelInfo {
    pub recipe: Recipe,
    pub input_shape: Vec<usize>,
    pub provenance: ImportedWeightsProvenance,
}

impl ImportedModelInfo {
    /// Preparation expected by the upstream weights. Apply it before recording image tensors.
    pub fn recommended_preprocessing(&self) -> Vec<flow_like_ml_core::PreprocessingStep> {
        let [3, height, width] = self.input_shape.as_slice() else {
            return Vec::new();
        };
        let imagenet = matches!(
            self.recipe,
            Recipe::ResNet18 { .. } | Recipe::DinoV2 { .. } | Recipe::RfDetr { .. }
        );
        vec![
            flow_like_ml_core::PreprocessingStep::Resize {
                width: *width,
                height: *height,
            },
            flow_like_ml_core::PreprocessingStep::Normalize {
                mean: if imagenet {
                    vec![0.485, 0.456, 0.406]
                } else {
                    vec![0.0; 3]
                },
                std: if imagenet {
                    vec![0.229, 0.224, 0.225]
                } else {
                    vec![1.0; 3]
                },
            },
        ]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema, PartialEq)]
pub struct FineTuneProvenance {
    pub source_model_sha256: String,
    pub source_recipe: Recipe,
    pub options: FineTuneOptions,
    #[serde(default)]
    pub imported: Option<ImportedWeightsProvenance>,
}

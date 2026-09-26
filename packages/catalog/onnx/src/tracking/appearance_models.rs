//! Built-in re-identification models for Extract Appearance. Each is downloaded on first use into
//! the node's cache folder, verified by SHA-256 and read from there afterwards. Only models whose
//! weights are licensed for commercial use are listed.

#[cfg(feature = "execute")]
use crate::onnx::{
    execution_providers::{configured_session_builder, ensure_ort_initialized},
    model_cache::{ModelSpec, REID_MODELS, validate_model_cache_dir, with_verified_models},
};
#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_model_provider::ml::ort::session::Session;
use flow_like_types::{Result, anyhow};

pub const CUSTOM_MODEL: &str = "custom";
pub const DEFAULT_MODEL: &str = "person-openvino-0270";

pub struct AppearanceModel {
    pub id: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
}

/// Intel OpenVINO Open Model Zoo person re-identification models, Apache-2.0. Both take
/// `[1,3,256,128]` RGB and return a 256-d embedding; they normalize the input themselves.
pub const APPEARANCE_MODELS: [AppearanceModel; 2] = [
    AppearanceModel {
        id: "person-openvino-0270",
        url: "https://storage.openvinotoolkit.org/repositories/open_model_zoo/2023.0/models_bin/1/person-reidentification-retail-0286/person-reidentification-retail-0270.onnx",
        sha256: "8f1c90c54cd60d3c1183ce37ea21d654a7cbdad6b2e9a38f56ac767deb09e111",
        size_bytes: 6_147_141,
    },
    AppearanceModel {
        id: "person-openvino-0265",
        url: "https://storage.openvinotoolkit.org/repositories/open_model_zoo/2023.0/models_bin/1/person-reidentification-retail-0277/person-reidentification-retail-0265.onnx",
        sha256: "e87c21b80d3dfafd6197c2830faeb1abf8d09e608b94cd90c3c7763b65bfbead",
        size_bytes: 9_632_724,
    },
];

/// Where Extract Appearance gets its model from, parsed from the `model` pin.
#[derive(Clone, Copy)]
pub enum ModelChoice {
    Builtin(&'static AppearanceModel),
    Custom,
}

impl ModelChoice {
    pub fn parse(value: &str) -> Result<Self> {
        if value == CUSTOM_MODEL {
            return Ok(Self::Custom);
        }
        APPEARANCE_MODELS
            .iter()
            .find(|model| model.id == value)
            .map(Self::Builtin)
            .ok_or_else(|| {
                anyhow!(
                    "Pin 'model' must be one of {}, got '{value}'",
                    model_options().join(", ")
                )
            })
    }
}

/// Values of the `model` dropdown: the built-in models, then `custom`.
pub fn model_options() -> Vec<String> {
    APPEARANCE_MODELS
        .iter()
        .map(|model| model.id.to_string())
        .chain([CUSTOM_MODEL.to_string()])
        .collect()
}

/// Downloads `model` into `cache_dir` unless a verified copy is already there, then builds an
/// ONNX session from it. Nothing is kept in memory after the caller drops the session.
#[cfg(feature = "execute")]
pub async fn load_session(
    context: &mut ExecutionContext,
    cache_dir: &FlowPath,
    model: &AppearanceModel,
) -> Result<Session> {
    validate_model_cache_dir(cache_dir, REID_MODELS.label)?;
    let spec = ModelSpec::new(
        &REID_MODELS,
        model.id,
        model.size_bytes,
        model.url,
        model.sha256,
    )?;
    ensure_ort_initialized()?;
    let id = model.id;
    with_verified_models(
        context,
        cache_dir,
        &[spec],
        "flowlike-reid-",
        move |paths| {
            let path = paths
                .first()
                .ok_or_else(|| anyhow!("No file was materialized for model '{id}'"))?;
            let bytes = std::fs::read(path)
                .map_err(|e| anyhow!("Failed to read cached model '{id}': {e}"))?;
            configured_session_builder()?
                .commit_from_memory(&bytes)
                .map_err(|e| anyhow!("Failed to load model '{id}': {e}"))
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onnx::model_cache::REID_MODELS;

    #[test]
    fn built_in_models_are_the_managed_reid_roles() {
        let ids: Vec<&str> = APPEARANCE_MODELS.iter().map(|model| model.id).collect();
        assert_eq!(ids, REID_MODELS.roles);
        assert_eq!(ids[0], DEFAULT_MODEL);
    }

    #[test]
    fn built_in_models_are_pinned() {
        for model in &APPEARANCE_MODELS {
            assert_eq!(model.sha256.len(), 64, "{}", model.id);
            assert!(model.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(
                model
                    .url
                    .starts_with("https://storage.openvinotoolkit.org/")
            );
            assert!(model.url.ends_with(".onnx"));
            assert!(model.size_bytes > 0);
        }
    }

    #[test]
    fn model_pin_values_parse() {
        assert!(matches!(
            ModelChoice::parse(DEFAULT_MODEL).unwrap(),
            ModelChoice::Builtin(model) if model.id == DEFAULT_MODEL
        ));
        assert!(matches!(
            ModelChoice::parse(CUSTOM_MODEL).unwrap(),
            ModelChoice::Custom
        ));
        let error = ModelChoice::parse("osnet").err().unwrap().to_string();
        assert!(
            error.contains("person-openvino-0270, person-openvino-0265, custom"),
            "{error}"
        );
    }
}

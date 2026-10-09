use crate::{
    BackendChoice, Error, ImportedModelInfo, ImportedWeightsProvenance, PretrainedWeightsMetadata,
    Recipe, Result, RfDetr, RfDetrConfig, RfDetrVariant, backend, engine,
    model::Model,
    pretrained::{SafeTensorReader, read_import_weights},
};
use std::path::Path;

/// Import official RF-DETR detection weights from SafeTensors or a torch.save ZIP.
/// Variant geometry and every tensor name and shape must match before an artifact is published.
pub fn import_rf_detr_weights(
    source_path: impl AsRef<Path>,
    artifact_dir: impl AsRef<Path>,
    variant: RfDetrVariant,
    metadata: &PretrainedWeightsMetadata,
) -> Result<ImportedModelInfo> {
    let (bytes, sha256, format) =
        read_import_weights(source_path.as_ref(), metadata, 1024 * 1024 * 1024)?;
    let mut reader = SafeTensorReader::new(&bytes)?;
    let shape = reader.shape("class_embed.weight")?;
    if shape.len() != 2 || shape[1] != 256 {
        return Err(Error::Invalid(
            "RF-DETR classification weight must have shape [classes, 256]".into(),
        ));
    }
    let config = RfDetrConfig {
        variant,
        classes: shape[0],
    };
    config.validate()?;
    let device = backend::device(&BackendChoice::Cpu, false)?;
    let _rng = crate::execution::lock(None)?;
    let model = RfDetr::new(&config, &device)?.import_safetensors(&mut reader, &device)?;
    reader.finish()?;
    let info = ImportedModelInfo {
        recipe: Recipe::RfDetr { config },
        input_shape: vec![3, variant.resolution(), variant.resolution()],
        provenance: ImportedWeightsProvenance {
            metadata: metadata.clone(),
            format: format!("rf_detr:{format}"),
            sha256,
            classification_head_pretrained: true,
        },
    };
    engine::write_imported_model(artifact_dir.as_ref(), &Model::RfDetr(model), &info)?;
    Ok(info)
}

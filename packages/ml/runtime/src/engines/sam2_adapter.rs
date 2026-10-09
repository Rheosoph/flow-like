use super::*;
use flow_like_ml_burn as burn;
use std::{fs, path::Path};

/// Register a native SAM image model without attaching classifier evaluation evidence.
pub fn publish_sam2_source(
    repository: &TrainingRepository,
    project_id: &str,
    directory: &Path,
    origin: Value,
    maximum_bytes: u64,
    at_ms: i64,
) -> Result<PretrainedSource> {
    repository.writable()?;
    let bytes =
        burn_adapter::pack_named(directory, burn::SAM2_MANIFEST_FILE, false, maximum_bytes)?;
    let snapshot = tempfile::tempdir()?;
    burn_adapter::unpack_named(
        &bytes,
        snapshot.path(),
        burn::SAM2_MANIFEST_FILE,
        maximum_bytes,
    )?;
    let state: Value =
        serde_json::from_slice(&fs::read(snapshot.path().join(burn::SAM2_MANIFEST_FILE))?)?;
    let manifest = source_manifest(&state)?;
    repository.register_pretrained_source(
        project_id,
        &bytes,
        manifest,
        origin,
        maximum_bytes,
        at_ms,
    )
}

pub fn predict_sam2_source(
    repository: &TrainingRepository,
    project_id: &str,
    source_id: &str,
    images: &TensorData,
    prompts: &[burn::Sam2Prompt],
    compute: &ComputeConfig,
) -> Result<burn::Sam2Prediction> {
    compute.validate().map_err(engine_error)?;
    let source = authorized_source(
        repository,
        project_id,
        source_id,
        compute.memory_limit_bytes,
    )?;
    let config: burn::Sam2Config = serde_json::from_value(source.manifest["config"].clone())?;
    images.validate(64 * 1024 * 1024).map_err(engine_error)?;
    let batch_shape = image_batch_shape(&images.shape, &[3, 1024, 1024])?;
    config
        .validate_input_shape(&batch_shape)
        .map_err(engine_error)?;
    if prompts.len() != batch_shape[0] {
        return Err(invalid("SAM needs one prompt per input image"));
    }
    for prompt in prompts {
        prompt.validate().map_err(engine_error)?;
    }
    admit_memory(
        &source,
        images.values.len(),
        prompts,
        batch_shape[0],
        None,
        compute,
    )?;
    let directory = unpack_source(repository, &source, compute.memory_limit_bytes)?;
    let predictor = burn::Sam2Predictor::load(directory.path(), burn_adapter::backend(compute)?)
        .map_err(engine_error)?;
    predictor
        .predict(
            &burn::TensorData {
                shape: batch_shape,
                values: images.values.clone(),
            },
            prompts,
        )
        .map_err(engine_error)
}

/// Run a separate SAM fine-tune and publish its completed weights as an immutable source.
/// Cancellation leaves the original source untouched and does not publish a partial result.
pub fn fine_tune_sam2_source(
    repository: &TrainingRepository,
    project_id: &str,
    source_id: &str,
    config: &burn::Sam2TrainingConfig,
    data: &burn::Sam2TrainingDataset,
    compute: &ComputeConfig,
    maximum_bytes: u64,
    cancellation: &burn::CancellationToken,
    progress: impl FnMut(burn::TrainingProgress),
    origin: Value,
    at_ms: i64,
) -> Result<Sam2FineTuneResult> {
    repository.writable()?;
    compute.validate().map_err(engine_error)?;
    config.validate().map_err(engine_error)?;
    data.validate(&config.model).map_err(engine_error)?;
    if !origin.is_object() {
        return Err(invalid("SAM fine-tune origin must be an object"));
    }
    if cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let source = authorized_source(
        repository,
        project_id,
        source_id,
        maximum_bytes.min(compute.memory_limit_bytes),
    )?;
    let source_config: burn::Sam2Config =
        serde_json::from_value(source.manifest["config"].clone())?;
    if config.model != source_config {
        return Err(invalid(
            "SAM fine-tuning must preserve the source architecture",
        ));
    }
    let values = data
        .images
        .values
        .len()
        .checked_add(data.masks.values.len())
        .ok_or_else(|| invalid("SAM dataset byte count overflow"))?;
    let batch = config.batch_size.min(data.images.shape[0]);
    admit_memory(
        &source,
        values,
        &data.prompts,
        batch,
        Some(config.freeze_image_encoder),
        compute,
    )?;
    let source_directory = unpack_source(
        repository,
        &source,
        maximum_bytes.min(compute.memory_limit_bytes),
    )?;
    let target_directory = tempfile::tempdir()?;
    let mut config = config.clone();
    config.backend = burn_adapter::backend(compute)?;
    let report = burn::train_sam2(
        &config,
        data,
        source_directory.path(),
        target_directory.path(),
        cancellation,
        progress,
    )
    .map_err(|error| match error {
        burn::Error::Cancelled => Error::Cancelled,
        other => engine_error(other),
    })?;
    if cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let mut origin = origin;
    origin["initialization_source_id"] = Value::String(source.id);
    origin["initialization_blob_sha256"] = Value::String(source.blob.sha256);
    let source = publish_sam2_source(
        repository,
        project_id,
        target_directory.path(),
        origin,
        maximum_bytes,
        at_ms,
    )?;
    Ok(Sam2FineTuneResult {
        source,
        report: serde_json::to_value(report)?,
    })
}

fn authorized_source(
    repository: &TrainingRepository,
    project: &str,
    id: &str,
    limit: u64,
) -> Result<PretrainedSource> {
    if project.trim().is_empty() || id.trim().is_empty() {
        return Err(invalid("SAM requires a project and source ID"));
    }
    let source = repository.get_pretrained_source(id)?;
    if source.project_id != project {
        return Err(invalid("SAM source belongs to a different project"));
    }
    if source.manifest["engine"] != "sam2"
        || source.manifest["format_version"].as_u64() != Some(1)
        || source.manifest["input_shape"] != serde_json::json!([3, 1024, 1024])
        || source.blob.bytes == 0
        || source.blob.bytes > limit
    {
        return Err(invalid("SAM source format or byte budget is incompatible"));
    }
    let config: burn::Sam2Config = serde_json::from_value(source.manifest["config"].clone())?;
    config.validate().map_err(engine_error)?;
    Ok(source)
}

fn unpack_source(
    repository: &TrainingRepository,
    source: &PretrainedSource,
    limit: u64,
) -> Result<tempfile::TempDir> {
    let bytes = repository.read_blob_limited(&source.blob, limit)?;
    let directory = tempfile::tempdir()?;
    burn_adapter::unpack_named(&bytes, directory.path(), burn::SAM2_MANIFEST_FILE, limit)?;
    let state: Value =
        serde_json::from_slice(&fs::read(directory.path().join(burn::SAM2_MANIFEST_FILE))?)?;
    if source_manifest(&state)? != source.manifest {
        return Err(invalid(
            "SAM source metadata differs from its verified model bundle",
        ));
    }
    Ok(directory)
}

fn source_manifest(state: &Value) -> Result<Value> {
    if state["version"].as_u64() != Some(1) {
        return Err(invalid("Unsupported SAM model state version"));
    }
    let model: burn::Sam2Config = serde_json::from_value(state["model"].clone())?;
    model.validate().map_err(engine_error)?;
    let provenance: burn::Sam2WeightsProvenance =
        serde_json::from_value(state["provenance"].clone())?;
    provenance.validate().map_err(engine_error)?;
    if !state["training"].is_null() {
        let training: burn::Sam2TrainingConfig = serde_json::from_value(state["training"].clone())?;
        training.validate().map_err(engine_error)?;
        if training.model != model {
            return Err(invalid("SAM model and training architecture differ"));
        }
    }
    for name in ["source_model_sha256", "dataset_sha256"] {
        if !state[name].is_null()
            && state[name].as_str().is_none_or(|digest| {
                digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            })
        {
            return Err(invalid("Invalid SAM source or dataset digest"));
        }
    }
    let _: burn::TrainingReport = serde_json::from_value(state["report"].clone())?;
    Ok(serde_json::json!({
        "engine":"sam2", "format_version":1, "config":model, "input_shape":[3,1024,1024],
        "provenance":provenance, "source_model_sha256":state["source_model_sha256"],
        "training":state["training"], "report":state["report"], "dataset_sha256":state["dataset_sha256"],
    }))
}

fn admit_memory(
    source: &PretrainedSource,
    values: usize,
    prompts: &[burn::Sam2Prompt],
    batch: usize,
    training: Option<bool>,
    compute: &ComputeConfig,
) -> Result<()> {
    // Reserve load-time parameter copies, optimizer state, and the fixed-resolution
    // Hiera attention workspace. Fine-tuning a frozen image encoder needs less tape.
    let (copies, workspace) = match training {
        None => (4u64, 1u64 << 30),
        Some(true) => (8, 2u64 << 30),
        Some(false) => (16, 6u64 << 30),
    };
    let prompt_values = prompts
        .iter()
        .try_fold(0usize, |sum, prompt| {
            sum.checked_add(prompt.mask.as_ref().map_or(0, |mask| mask.values.len()))
                .and_then(|n| n.checked_add(prompt.points.len().checked_mul(4)?))
        })
        .ok_or_else(|| invalid("SAM prompt memory estimate overflows"))?;
    let estimate = source
        .blob
        .bytes
        .checked_mul(copies)
        .and_then(|n| n.checked_add((values.checked_add(prompt_values)? as u64).checked_mul(8)?))
        .and_then(|n| n.checked_add(workspace.checked_mul(batch as u64)?))
        .ok_or_else(|| invalid("SAM memory estimate overflows"))?;
    if estimate > compute.memory_limit_bytes {
        return Err(invalid(format!(
            "SAM needs an estimated {estimate} bytes, exceeding the configured {} byte memory limit",
            compute.memory_limit_bytes
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(repository: &TrainingRepository, directory: &Path) -> PretrainedSource {
        let model = serde_json::json!({"variant":"hiera_tiny"});
        fs::create_dir_all(directory.join("sam2-checkpoint-test")).unwrap();
        fs::write(
            directory.join("sam2-checkpoint-test/model.bpk"),
            b"test-model-bytes",
        )
        .unwrap();
        let state = serde_json::json!({
            "version":1, "model":model,
            "provenance": {"metadata":{"source":"test://sam2","license":"Apache-2.0"}, "format":"safetensors_f32", "sha256":"a".repeat(64)},
            "source_model_sha256":null, "training":null, "dataset_sha256":null,
            "checkpoint":"sam2-checkpoint-test", "next_epoch":0, "next_batch":0,
            "report":{"completed_epochs":0,"steps":0,"initial_loss":0.0,"final_loss":0.0,"validation_loss":null,"history":[]}
        });
        fs::write(
            directory.join(burn::SAM2_MANIFEST_FILE),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        publish_sam2_source(
            repository,
            "project-a",
            directory,
            serde_json::json!({"source":"test"}),
            1024 * 1024,
            10,
        )
        .unwrap()
    }

    #[test]
    fn sam_sources_keep_checkpoint_names_and_project_authorization() {
        let root = tempfile::tempdir().unwrap();
        let repository = TrainingRepository::open(root.path().join("training.sqlite")).unwrap();
        let source = fixture(&repository, &root.path().join("model"));
        assert_eq!(source.manifest["engine"], "sam2");
        assert_eq!(
            repository
                .list_pretrained_sources("project-a", 10)
                .unwrap()
                .len(),
            1
        );
        assert!(authorized_source(&repository, "project-b", &source.id, 1024 * 1024).is_err());
        assert!(
            authorized_source(&repository, "project-a", &source.id, source.blob.bytes - 1).is_err()
        );
        let unpacked = unpack_source(&repository, &source, 1024 * 1024).unwrap();
        let state: Value = serde_json::from_slice(
            &fs::read(unpacked.path().join(burn::SAM2_MANIFEST_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(state["checkpoint"], "sam2-checkpoint-runtime");
        assert_eq!(
            fs::read(unpacked.path().join("sam2-checkpoint-runtime/model.bpk")).unwrap(),
            b"test-model-bytes"
        );
        let mut forged = source.clone();
        forged.manifest["provenance"]["sha256"] = Value::String("b".repeat(64));
        assert!(unpack_source(&repository, &forged, 1024 * 1024).is_err());
        let blob_path = root
            .path()
            .join("training.artifacts")
            .join(&source.blob.path);
        let mut bytes = fs::read(&blob_path).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        fs::write(blob_path, bytes).unwrap();
        assert!(unpack_source(&repository, &source, 1024 * 1024).is_err());
    }

    #[test]
    fn sam_prediction_accepts_chw_with_one_prompt_and_preserves_shape_and_memory_checks() {
        let root = tempfile::tempdir().unwrap();
        let repository = TrainingRepository::open(root.path().join("training.sqlite")).unwrap();
        let source = fixture(&repository, &root.path().join("model"));
        let compute = ComputeConfig {
            memory_limit_bytes: 32 * 1024 * 1024,
            ..Default::default()
        };
        let mut image = TensorData {
            shape: vec![3, 1024, 1024],
            values: vec![0.; 3 * 1024 * 1024],
        };
        assert_eq!(
            image_batch_shape(&image.shape, &[3, 1024, 1024]).unwrap(),
            vec![1, 3, 1024, 1024]
        );
        assert!(
            predict_sam2_source(&repository, "project-a", &source.id, &image, &[], &compute)
                .unwrap_err()
                .to_string()
                .contains("one prompt per input image")
        );
        let prompts = [burn::Sam2Prompt::default()];
        let unbatched = predict_sam2_source(
            &repository,
            "project-a",
            &source.id,
            &image,
            &prompts,
            &compute,
        )
        .unwrap_err()
        .to_string();
        assert!(unbatched.contains("SAM needs an estimated"));
        image.shape.insert(0, 1);
        let batched = predict_sam2_source(
            &repository,
            "project-a",
            &source.id,
            &image,
            &prompts,
            &compute,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            unbatched, batched,
            "CHW and single-image NCHW must reserve the same memory"
        );
        image.shape = vec![1024, 1024, 3];
        assert!(
            predict_sam2_source(
                &repository,
                "project-a",
                &source.id,
                &image,
                &prompts,
                &compute
            )
            .unwrap_err()
            .to_string()
            .contains("image input contract")
        );
        assert!(image_batch_shape(&[0, 3, 1024, 1024], &[3, 1024, 1024]).is_err());
        assert!(image_batch_shape(&[1, 3, 512, 1024], &[3, 1024, 1024]).is_err());
    }

    #[test]
    fn sam_memory_admission_accounts_for_training_batch_and_tape() {
        let root = tempfile::tempdir().unwrap();
        let repository = TrainingRepository::open(root.path().join("training.sqlite")).unwrap();
        let mut source = fixture(&repository, &root.path().join("model"));
        source.blob.bytes = 160 * 1024 * 1024;
        let compute = ComputeConfig {
            memory_limit_bytes: 4 * 1024 * 1024 * 1024,
            ..Default::default()
        };
        let values = 3 * 1024 * 1024 + 256 * 256;
        assert!(admit_memory(&source, values, &[], 1, None, &compute).is_ok());
        assert!(admit_memory(&source, values, &[], 1, Some(true), &compute).is_ok());
        assert!(admit_memory(&source, values, &[], 1, Some(false), &compute).is_err());
        assert!(admit_memory(&source, values, &[], 1024, Some(true), &compute).is_err());
        assert!(admit_memory(&source, usize::MAX, &[], 1, Some(true), &compute).is_err());
    }
}

use crate::{
    Error, ImportedModelInfo, ImportedWeightsProvenance, PretrainedWeightsMetadata, Recipe, Result,
    backend, engine, model::Model, vision::ResNet18,
};
use burn::{
    module::{Param, RunningState},
    nn::{BatchNorm, LayerNorm, Linear, conv::Conv2d},
    tensor::{Device, Tensor, TensorData},
};
use safetensors::{Dtype, SafeTensors};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs::File, io::Read, path::Path};

pub(crate) struct SafeTensorReader<'a> {
    tensors: SafeTensors<'a>,
    used: HashSet<String>,
}
impl<'a> SafeTensorReader<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Result<Self> {
        Ok(Self {
            tensors: SafeTensors::deserialize(bytes)
                .map_err(|error| Error::Record(format!("Invalid SafeTensors weights: {error}")))?,
            used: HashSet::new(),
        })
    }
    pub(crate) fn shape(&self, name: &str) -> Result<Vec<usize>> {
        self.tensors
            .tensor(name)
            .map(|tensor| tensor.shape().to_vec())
            .map_err(|error| Error::Record(format!("Pretrained tensor {name}: {error}")))
    }
    pub(crate) fn has(&self, name: &str) -> bool {
        self.tensors.names().contains(&name)
    }
    #[cfg(test)]
    pub(crate) fn names(&self) -> Vec<String> {
        let mut names = self
            .tensors
            .names()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        names.sort_unstable();
        names
    }
    pub(crate) fn raw(&mut self, name: &str) -> Result<(Dtype, Vec<usize>, Vec<u8>)> {
        let tensor = self
            .tensors
            .tensor(name)
            .map_err(|error| Error::Record(format!("Pretrained tensor {name}: {error}")))?;
        let value = (
            tensor.dtype(),
            tensor.shape().to_vec(),
            tensor.data().to_vec(),
        );
        self.used.insert(name.to_string());
        Ok(value)
    }
    pub(crate) fn tensor<const D: usize>(
        &mut self,
        name: &str,
        expected: [usize; D],
        device: &Device,
    ) -> Result<Tensor<D>> {
        let source = self
            .tensors
            .tensor(name)
            .map_err(|error| Error::Record(format!("Pretrained tensor {name}: {error}")))?;
        if source.dtype() != Dtype::F32 || source.shape() != expected {
            return Err(Error::Invalid(format!(
                "Pretrained tensor {name} must be F32 with shape {expected:?}; found {:?} {:?}",
                source.dtype(),
                source.shape()
            )));
        }
        let values = source
            .data()
            .chunks_exact(4)
            .map(|value| f32::from_le_bytes(value.try_into().expect("four-byte chunk")))
            .collect::<Vec<_>>();
        if values.iter().any(|value| !value.is_finite()) {
            return Err(Error::Invalid(format!(
                "Pretrained tensor {name} contains non-finite values"
            )));
        }
        self.used.insert(name.to_string());
        Ok(Tensor::from_data(TensorData::new(values, expected), device))
    }
    pub(crate) fn linear_weight(
        &mut self,
        name: &str,
        input: usize,
        output: usize,
        device: &Device,
    ) -> Result<Tensor<2>> {
        // PyTorch stores [output, input]; Burn Linear stores [input, output].
        Ok(self.tensor(name, [output, input], device)?.transpose())
    }
    pub(crate) fn linear(
        &mut self,
        mut layer: Linear,
        prefix: &str,
        device: &Device,
    ) -> Result<Linear> {
        let [input, output] = layer.weight.val().dims();
        layer.weight = Param::from_tensor(self.linear_weight(
            &format!("{prefix}.weight"),
            input,
            output,
            device,
        )?);
        if layer.bias.is_some() {
            layer.bias = Some(Param::from_tensor(self.tensor(
                &format!("{prefix}.bias"),
                [output],
                device,
            )?));
        }
        Ok(layer)
    }
    pub(crate) fn conv2d(
        &mut self,
        mut layer: Conv2d,
        prefix: &str,
        device: &Device,
    ) -> Result<Conv2d> {
        let shape = layer.weight.val().dims();
        layer.weight =
            Param::from_tensor(self.tensor(&format!("{prefix}.weight"), shape, device)?);
        if layer.bias.is_some() {
            layer.bias = Some(Param::from_tensor(self.tensor(
                &format!("{prefix}.bias"),
                [shape[0]],
                device,
            )?));
        }
        Ok(layer)
    }
    pub(crate) fn batch_norm(
        &mut self,
        mut layer: BatchNorm,
        prefix: &str,
        device: &Device,
    ) -> Result<BatchNorm> {
        let shape = layer.gamma.val().dims();
        layer.gamma =
            Param::from_tensor(self.tensor(&format!("{prefix}.weight"), shape, device)?);
        layer.beta = Param::from_tensor(self.tensor(&format!("{prefix}.bias"), shape, device)?);
        layer.running_mean =
            RunningState::new(self.tensor(&format!("{prefix}.running_mean"), shape, device)?);
        let variance = self.tensor(&format!("{prefix}.running_var"), shape, device)?;
        if variance
            .clone()
            .into_data()
            .iter::<f32>()
            .any(|value| value < 0.0)
        {
            return Err(Error::Invalid(format!(
                "{prefix}.running_var contains a negative variance"
            )));
        }
        layer.running_var = RunningState::new(variance);
        self.optional_batch_counter(&format!("{prefix}.num_batches_tracked"))?;
        Ok(layer)
    }
    pub(crate) fn layer_norm(
        &mut self,
        mut layer: LayerNorm,
        prefix: &str,
        device: &Device,
    ) -> Result<LayerNorm> {
        let shape = layer.gamma.val().dims();
        layer.gamma =
            Param::from_tensor(self.tensor(&format!("{prefix}.weight"), shape, device)?);
        if layer.beta.is_some() {
            layer.beta = Some(Param::from_tensor(self.tensor(
                &format!("{prefix}.bias"),
                shape,
                device,
            )?));
        }
        Ok(layer)
    }
    pub(crate) fn optional_batch_counter(&mut self, name: &str) -> Result<()> {
        if !self.tensors.names().contains(&name) {
            return Ok(());
        }
        let tensor = self
            .tensors
            .tensor(name)
            .map_err(|error| Error::Record(error.to_string()))?;
        if tensor.dtype() != Dtype::I64
            || !tensor.shape().is_empty()
            || i64::from_le_bytes(
                tensor
                    .data()
                    .try_into()
                    .map_err(|_| Error::Invalid("Invalid batch counter".into()))?,
            ) < 0
        {
            return Err(Error::Invalid(format!(
                "Pretrained tensor {name} must be a nonnegative I64 scalar"
            )));
        }
        self.used.insert(name.to_string());
        Ok(())
    }
    pub(crate) fn finish(self) -> Result<()> {
        let mut unknown = self
            .tensors
            .names()
            .into_iter()
            .filter(|name| !self.used.contains(*name))
            .collect::<Vec<_>>();
        unknown.sort_unstable();
        if !unknown.is_empty() {
            return Err(Error::Invalid(format!(
                "Unrecognized pretrained tensors: {:?}",
                &unknown[..unknown.len().min(8)]
            )));
        }
        Ok(())
    }
}

pub(crate) fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(crate) fn read_weights(
    path: &Path,
    metadata: &PretrainedWeightsMetadata,
    maximum_bytes: u64,
) -> Result<(Vec<u8>, String)> {
    metadata.validate()?;
    let mut bytes = Vec::new();
    File::open(path)?
        .take(maximum_bytes + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum_bytes {
        return Err(Error::Invalid(
            "Pretrained weights exceed the import byte limit".into(),
        ));
    }
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if metadata
        .expected_sha256
        .as_ref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&digest))
    {
        return Err(Error::Invalid(
            "Pretrained weight SHA256 does not match the expected digest".into(),
        ));
    }
    Ok((bytes, digest))
}

pub(crate) fn read_import_weights(
    path: &Path,
    metadata: &PretrainedWeightsMetadata,
    maximum_bytes: u64,
) -> Result<(Vec<u8>, String, String)> {
    let (bytes, digest) = read_weights(path, metadata, maximum_bytes)?;
    if bytes.starts_with(b"PK\x03\x04") {
        let (bytes, format) = crate::pytorch::convert_pytorch_weights(&bytes, maximum_bytes)?;
        Ok((bytes, digest, format))
    } else {
        Ok((bytes, digest, "safetensors_f32".into()))
    }
}

/// Import canonical torchvision ResNet-18 F32 weights from SafeTensors or a torch.save ZIP.
/// Keys must match torchvision's state_dict; RGB inputs use ImageNet normalization.
pub fn import_torchvision_resnet18_safetensors(
    source_path: impl AsRef<Path>,
    artifact_dir: impl AsRef<Path>,
    metadata: &PretrainedWeightsMetadata,
) -> Result<ImportedModelInfo> {
    let (bytes, sha256, format) =
        read_import_weights(source_path.as_ref(), metadata, 256 * 1024 * 1024)?;
    let mut reader = SafeTensorReader::new(&bytes)?;
    let head = reader.shape("fc.weight")?;
    if head.len() != 2 || head[1] != 512 {
        return Err(Error::Invalid(
            "torchvision ResNet-18 fc.weight must be [classes, 512]".into(),
        ));
    }
    let recipe = Recipe::ResNet18 {
        input_channels: 3,
        classes: head[0],
        base_channels: 64,
    };
    recipe.validate()?;
    let device = backend::device(&crate::BackendChoice::Cpu, false)?;
    let _rng = crate::execution::lock(None)?;
    let model = ResNet18::new(3, head[0], 64, &device).import_torchvision(&mut reader, &device)?;
    reader.finish()?;
    let info = ImportedModelInfo {
        recipe,
        input_shape: vec![3, 224, 224],
        provenance: ImportedWeightsProvenance {
            metadata: metadata.clone(),
            format: format!("torchvision_resnet18:{format}"),
            sha256,
            classification_head_pretrained: true,
        },
    };
    engine::write_imported_model(artifact_dir.as_ref(), &Model::ResNet18(model), &info)?;
    Ok(info)
}

/// Import an official DINOv2 backbone. Its newly initialized classifier must be trained separately.
pub fn import_dinov2_safetensors(
    source_path: impl AsRef<Path>,
    artifact_dir: impl AsRef<Path>,
    variant: crate::DinoV2Variant,
    metadata: &PretrainedWeightsMetadata,
) -> Result<ImportedModelInfo> {
    let (bytes, sha256, format) =
        read_import_weights(source_path.as_ref(), metadata, 512 * 1024 * 1024)?;
    let mut reader = SafeTensorReader::new(&bytes)?;
    let config = crate::DinoV2Config {
        variant,
        classes: 1000,
    };
    let device = backend::device(&crate::BackendChoice::Cpu, false)?;
    let _rng = crate::execution::lock(None)?;
    let model = crate::DinoV2::new(&config, &device)?.import_safetensors(&mut reader, &device)?;
    reader.finish()?;
    let info = ImportedModelInfo {
        recipe: Recipe::DinoV2 { config },
        input_shape: vec![3, 518, 518],
        provenance: ImportedWeightsProvenance {
            metadata: metadata.clone(),
            format: format!("dinov2:{format}"),
            sha256,
            classification_head_pretrained: false,
        },
    };
    engine::write_imported_model(artifact_dir.as_ref(), &Model::DinoV2(model), &info)?;
    Ok(info)
}

/// Import official RT-DETRv2 tensors, selecting EMA weights when the checkpoint includes them.
pub fn import_rtdetr_v2_weights(
    source_path: impl AsRef<Path>,
    artifact_dir: impl AsRef<Path>,
    backbone: crate::RtdetrV2Backbone,
    metadata: &PretrainedWeightsMetadata,
) -> Result<ImportedModelInfo> {
    let (bytes, sha256, format) =
        read_import_weights(source_path.as_ref(), metadata, 1024 * 1024 * 1024)?;
    let mut reader = SafeTensorReader::new(&bytes)?;
    let head = reader.shape("decoder.enc_score_head.weight")?;
    if head.len() != 2 || head[1] != 256 {
        return Err(Error::Invalid(
            "RT-DETRv2 encoder classification weight must be [classes, 256]".into(),
        ));
    }
    let config = crate::RtdetrV2Config {
        backbone,
        classes: head[0],
        queries: 300,
    };
    config.validate()?;
    let device = backend::device(&crate::BackendChoice::Cpu, false)?;
    let _rng = crate::execution::lock(None)?;
    let model = crate::RtdetrV2::new(&config, &device)?.import_safetensors(&mut reader, &device)?;
    reader.finish()?;
    let info = ImportedModelInfo {
        recipe: Recipe::RtdetrV2 { config },
        input_shape: vec![3, 640, 640],
        provenance: ImportedWeightsProvenance {
            metadata: metadata.clone(),
            format: format!("rtdetr_v2:{format}"),
            sha256,
            classification_head_pretrained: true,
        },
    };
    engine::write_imported_model(artifact_dir.as_ref(), &Model::RtdetrV2(model), &info)?;
    Ok(info)
}

/// Import D-FINE Nano tensors with their source digest and original checkpoint provenance.
pub fn import_dfine_nano_weights(
    source_path: impl AsRef<Path>,
    artifact_dir: impl AsRef<Path>,
    metadata: &PretrainedWeightsMetadata,
) -> Result<ImportedModelInfo> {
    let (bytes, sha256, format) =
        read_import_weights(source_path.as_ref(), metadata, 256 * 1024 * 1024)?;
    let mut reader = SafeTensorReader::new(&bytes)?;
    let head = reader.shape("decoder.enc_score_head.weight")?;
    if head.len() != 2 || head[1] != 128 {
        return Err(Error::Invalid(
            "D-FINE Nano encoder classification weight must be [classes, 128]".into(),
        ));
    }
    let config = crate::DfineConfig {
        classes: head[0],
        queries: 300,
    };
    config.validate()?;
    let device = backend::device(&crate::BackendChoice::Cpu, false)?;
    let _rng = crate::execution::lock(None)?;
    let model =
        crate::DfineNano::new(&config, &device)?.import_safetensors(&mut reader, &device)?;
    reader.finish()?;
    let info = ImportedModelInfo {
        recipe: Recipe::DfineNano { config },
        input_shape: vec![3, 640, 640],
        provenance: ImportedWeightsProvenance {
            metadata: metadata.clone(),
            format: format!("dfine_nano:{format}"),
            sha256,
            classification_head_pretrained: true,
        },
    };
    engine::write_imported_model(artifact_dir.as_ref(), &Model::DfineNano(model), &info)?;
    Ok(info)
}

#[cfg(all(test, feature = "cpu"))]
#[path = "pretrained_tests.rs"]
mod tests;

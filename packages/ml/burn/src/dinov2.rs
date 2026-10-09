// Copyright (c) Meta Platforms, Inc. and affiliates.
// Native adaptation of DINOv2 under Apache-2.0; see licenses/DINOv2.
use crate::{DinoV2Config, Error, Result, pretrained::SafeTensorReader};
use burn::{
    module::{Module, Param},
    nn::{
        Initializer, LayerNorm, LayerNormConfig, Linear, LinearConfig,
        conv::{Conv2d, Conv2dConfig},
    },
    tensor::{
        Device, Tensor, TensorData as BurnData,
        activation::{gelu, softmax},
    },
};

#[derive(Clone, Debug)]
pub struct DinoV2Features {
    pub cls: Tensor<2>,
    pub patches: Tensor<3>,
    pub grid: [usize; 2],
}
impl DinoV2Features {
    /// Return the row-major patch tokens as an NCHW feature map.
    pub fn feature_map(&self) -> Tensor<4> {
        let [batch, _, embedding] = self.patches.dims();
        self.patches
            .clone()
            .reshape([batch, self.grid[0], self.grid[1], embedding])
            .permute([0, 3, 1, 2])
    }
}

#[derive(Clone, Copy, Debug)]
struct Dimensions {
    embedding: usize,
    depth: usize,
    heads: usize,
    patch: usize,
    grid: usize,
}
impl From<&DinoV2Config> for Dimensions {
    fn from(config: &DinoV2Config) -> Self {
        Self {
            embedding: config.variant.embedding_dim(),
            depth: config.variant.depth(),
            heads: config.variant.heads(),
            patch: DinoV2Config::PATCH_SIZE,
            grid: DinoV2Config::PRETRAINED_GRID,
        }
    }
}

#[derive(Module, Debug)]
struct Attention {
    qkv: Linear,
    projection: Linear,
    heads: usize,
}
impl Attention {
    fn new(embedding: usize, heads: usize, device: &Device) -> Self {
        Self {
            qkv: linear(embedding, 3 * embedding, device),
            projection: linear(embedding, embedding, device),
            heads,
        }
    }
    fn forward(&self, input: Tensor<3>) -> Tensor<3> {
        let [batch, tokens, embedding] = input.dims();
        let head_dim = embedding / self.heads;
        let qkv = self
            .qkv
            .forward(input)
            .reshape([batch, tokens, 3, self.heads, head_dim]);
        let part = |index| {
            qkv.clone()
                .slice([
                    0..batch,
                    0..tokens,
                    index..index + 1,
                    0..self.heads,
                    0..head_dim,
                ])
                .reshape([batch, tokens, self.heads, head_dim])
                .permute([0, 2, 1, 3])
        };
        let query = part(0) / (head_dim as f64).sqrt();
        let attention = softmax(query.matmul(part(1).swap_dims(2, 3)), 3);
        self.projection.forward(
            attention
                .matmul(part(2))
                .permute([0, 2, 1, 3])
                .reshape([batch, tokens, embedding]),
        )
    }
}

#[derive(Module, Debug)]
pub(crate) struct Block {
    norm1: LayerNorm,
    attention: Attention,
    scale1: Param<Tensor<1>>,
    norm2: LayerNorm,
    fc1: Linear,
    fc2: Linear,
    scale2: Param<Tensor<1>>,
}
impl Block {
    pub(crate) fn new(embedding: usize, heads: usize, device: &Device) -> Self {
        Self {
            norm1: norm(embedding, device),
            attention: Attention::new(embedding, heads, device),
            scale1: Initializer::Ones.init([embedding], device),
            norm2: norm(embedding, device),
            fc1: linear(embedding, embedding * 4, device),
            fc2: linear(embedding * 4, embedding, device),
            scale2: Initializer::Ones.init([embedding], device),
        }
    }
    pub(crate) fn forward(&self, input: Tensor<3>) -> Tensor<3> {
        let embedding = input.dims()[2];
        let attended = self.attention.forward(self.norm1.forward(input.clone()));
        let input = input + attended * self.scale1.val().reshape([1, 1, embedding]);
        let hidden = self
            .fc2
            .forward(gelu(self.fc1.forward(self.norm2.forward(input.clone()))));
        input + hidden * self.scale2.val().reshape([1, 1, embedding])
    }
    pub(crate) fn import_hugging_face(
        &mut self,
        reader: &mut SafeTensorReader<'_>,
        prefix: &str,
        device: &Device,
    ) -> Result<()> {
        let embedding = self.norm1.gamma.val().dims()[0];
        import_norm(
            &mut self.norm1,
            reader,
            &format!("{prefix}.norm1"),
            embedding,
            device,
        )?;
        let mut weights = Vec::with_capacity(3);
        let mut biases = Vec::with_capacity(3);
        for projection in ["query", "key", "value"] {
            let name = format!("{prefix}.attention.attention.{projection}");
            weights.push(reader.linear_weight(
                &format!("{name}.weight"),
                embedding,
                embedding,
                device,
            )?);
            biases.push(reader.tensor(&format!("{name}.bias"), [embedding], device)?);
        }
        self.attention.qkv.weight = Param::from_tensor(Tensor::cat(weights, 1));
        self.attention.qkv.bias = Some(Param::from_tensor(Tensor::cat(biases, 0)));
        import_linear(
            &mut self.attention.projection,
            reader,
            &format!("{prefix}.attention.output.dense"),
            embedding,
            embedding,
            device,
        )?;
        self.scale1 = Param::from_tensor(reader.tensor(
            &format!("{prefix}.layer_scale1.lambda1"),
            [embedding],
            device,
        )?);
        import_norm(
            &mut self.norm2,
            reader,
            &format!("{prefix}.norm2"),
            embedding,
            device,
        )?;
        import_linear(
            &mut self.fc1,
            reader,
            &format!("{prefix}.mlp.fc1"),
            embedding,
            4 * embedding,
            device,
        )?;
        import_linear(
            &mut self.fc2,
            reader,
            &format!("{prefix}.mlp.fc2"),
            4 * embedding,
            embedding,
            device,
        )?;
        self.scale2 = Param::from_tensor(reader.tensor(
            &format!("{prefix}.layer_scale2.lambda1"),
            [embedding],
            device,
        )?);
        Ok(())
    }
}

/// DINOv2 ViT-S/14 or ViT-B/14 with the Meta no-register backbone layout.
/// Checkpoint names and positional interpolation follow the Apache-2.0 implementation:
/// https://github.com/facebookresearch/dinov2/blob/main/dinov2/models/vision_transformer.py
/// Inputs are RGB NCHW tensors normalized with the ImageNet mean and standard deviation.
#[derive(Module, Debug)]
pub struct DinoV2 {
    patch_embedding: Conv2d,
    cls_token: Param<Tensor<3>>,
    position: Param<Tensor<3>>,
    mask_token: Param<Tensor<2>>,
    blocks: Vec<Block>,
    norm: LayerNorm,
    classifier: Linear,
    #[module(skip)]
    dimensions: Dimensions,
}
impl DinoV2 {
    pub fn new(config: &DinoV2Config, device: &Device) -> Result<Self> {
        config.validate()?;
        Ok(Self::with_dimensions(config.classes, config.into(), device))
    }
    fn with_dimensions(classes: usize, dimensions: Dimensions, device: &Device) -> Self {
        let Dimensions {
            embedding,
            depth,
            heads,
            patch,
            grid,
        } = dimensions;
        Self {
            patch_embedding: Conv2dConfig::new([3, embedding], [patch, patch])
                .with_stride([patch, patch])
                .init(device),
            cls_token: Initializer::Normal {
                mean: 0.0,
                std: 1e-6,
            }
            .init([1, 1, embedding], device),
            position: Initializer::Normal {
                mean: 0.0,
                std: 0.02,
            }
            .init([1, 1 + grid * grid, embedding], device),
            mask_token: Initializer::Zeros.init([1, embedding], device),
            blocks: (0..depth)
                .map(|_| Block::new(embedding, heads, device))
                .collect(),
            norm: norm(embedding, device),
            classifier: linear(embedding, classes, device),
            dimensions,
        }
    }

    pub fn forward(&self, input: Tensor<4>) -> Tensor<2> {
        self.classifier.forward(self.features(input).cls)
    }

    pub fn features(&self, input: Tensor<4>) -> DinoV2Features {
        let (mut tokens, grid) = self.prepare_tokens(input);
        for block in &self.blocks {
            tokens = block.forward(tokens);
        }
        split_features(self.norm.forward(tokens), grid)
    }

    /// Take zero-based block outputs in ascending order, optionally applying the final norm.
    pub fn intermediate_features(
        &self,
        input: Tensor<4>,
        layers: &[usize],
        normalize: bool,
    ) -> Result<Vec<DinoV2Features>> {
        if layers.is_empty()
            || layers.iter().any(|index| *index >= self.blocks.len())
            || layers.windows(2).any(|pair| pair[0] >= pair[1])
        {
            return Err(Error::Invalid(
                "DINOv2 feature layers must be unique, ascending block indices".into(),
            ));
        }
        let (mut tokens, grid) = self.prepare_tokens(input);
        let mut features = Vec::with_capacity(layers.len());
        for (index, block) in self.blocks.iter().enumerate() {
            tokens = block.forward(tokens);
            if layers.contains(&index) {
                let output = if normalize {
                    self.norm.forward(tokens.clone())
                } else {
                    tokens.clone()
                };
                features.push(split_features(output, grid));
            }
            if features.len() == layers.len() {
                break;
            }
        }
        Ok(features)
    }

    pub fn reset_classifier(mut self, classes: usize, device: &Device) -> Self {
        assert!(
            (1..=65_536).contains(&classes),
            "Invalid DINOv2 class count"
        );
        self.classifier = linear(self.dimensions.embedding, classes, device);
        self
    }

    pub fn configure_fine_tuning(self, freeze_backbone: bool) -> Self {
        if !freeze_backbone {
            return self.unfreeze();
        }
        let mut frozen = self.freeze();
        frozen.classifier = frozen.classifier.unfreeze();
        frozen
    }

    /// Import a standard Meta backbone state dict. The supervised head remains freshly initialized.
    pub(crate) fn import_safetensors(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        if reader.shape("embeddings.cls_token").is_ok() {
            return self.import_hugging_face(reader, device);
        }
        let Dimensions {
            embedding,
            patch,
            grid,
            ..
        } = self.dimensions;
        self.cls_token =
            Param::from_tensor(reader.tensor("cls_token", [1, 1, embedding], device)?);
        self.position = Param::from_tensor(reader.tensor(
            "pos_embed",
            [1, 1 + grid * grid, embedding],
            device,
        )?);
        self.mask_token =
            Param::from_tensor(reader.tensor("mask_token", [1, embedding], device)?);
        self.patch_embedding.weight = Param::from_tensor(reader.tensor(
            "patch_embed.proj.weight",
            [embedding, 3, patch, patch],
            device,
        )?);
        self.patch_embedding.bias = Some(Param::from_tensor(reader.tensor(
            "patch_embed.proj.bias",
            [embedding],
            device,
        )?));
        for (index, block) in self.blocks.iter_mut().enumerate() {
            let prefix = format!("blocks.{index}");
            import_norm(
                &mut block.norm1,
                reader,
                &format!("{prefix}.norm1"),
                embedding,
                device,
            )?;
            import_linear(
                &mut block.attention.qkv,
                reader,
                &format!("{prefix}.attn.qkv"),
                embedding,
                3 * embedding,
                device,
            )?;
            import_linear(
                &mut block.attention.projection,
                reader,
                &format!("{prefix}.attn.proj"),
                embedding,
                embedding,
                device,
            )?;
            block.scale1 = Param::from_tensor(reader.tensor(
                &format!("{prefix}.ls1.gamma"),
                [embedding],
                device,
            )?);
            import_norm(
                &mut block.norm2,
                reader,
                &format!("{prefix}.norm2"),
                embedding,
                device,
            )?;
            import_linear(
                &mut block.fc1,
                reader,
                &format!("{prefix}.mlp.fc1"),
                embedding,
                4 * embedding,
                device,
            )?;
            import_linear(
                &mut block.fc2,
                reader,
                &format!("{prefix}.mlp.fc2"),
                4 * embedding,
                embedding,
                device,
            )?;
            block.scale2 = Param::from_tensor(reader.tensor(
                &format!("{prefix}.ls2.gamma"),
                [embedding],
                device,
            )?);
        }
        import_norm(&mut self.norm, reader, "norm", embedding, device)?;
        Ok(self)
    }

    fn import_hugging_face(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        // facebook/dinov2-small and facebook/dinov2-base preserve the original
        // backbone weights, with q/k/v split along the output dimension.
        let Dimensions {
            embedding,
            patch,
            grid,
            ..
        } = self.dimensions;
        self.cls_token =
            Param::from_tensor(reader.tensor("embeddings.cls_token", [1, 1, embedding], device)?);
        self.position = Param::from_tensor(reader.tensor(
            "embeddings.position_embeddings",
            [1, 1 + grid * grid, embedding],
            device,
        )?);
        self.mask_token =
            Param::from_tensor(reader.tensor("embeddings.mask_token", [1, embedding], device)?);
        self.patch_embedding.weight = Param::from_tensor(reader.tensor(
            "embeddings.patch_embeddings.projection.weight",
            [embedding, 3, patch, patch],
            device,
        )?);
        self.patch_embedding.bias = Some(Param::from_tensor(reader.tensor(
            "embeddings.patch_embeddings.projection.bias",
            [embedding],
            device,
        )?));
        for (index, block) in self.blocks.iter_mut().enumerate() {
            block.import_hugging_face(reader, &format!("encoder.layer.{index}"), device)?;
        }
        import_norm(&mut self.norm, reader, "layernorm", embedding, device)?;
        Ok(self)
    }

    fn prepare_tokens(&self, input: Tensor<4>) -> (Tensor<3>, [usize; 2]) {
        let [batch, channels, height, width] = input.dims();
        let Dimensions {
            embedding, patch, ..
        } = self.dimensions;
        assert!(
            batch > 0
                && channels == 3
                && height > 0
                && width > 0
                && height.is_multiple_of(patch)
                && width.is_multiple_of(patch),
            "DINOv2 input must be nonempty RGB NCHW with dimensions divisible by its patch size"
        );
        let grid = [height / patch, width / patch];
        assert!(
            grid[0]
                .checked_mul(grid[1])
                .is_some_and(|count| count <= 4096),
            "DINOv2 input exceeds 4096 patches per image"
        );
        let patches = self
            .patch_embedding
            .forward(input)
            .reshape([batch, embedding, grid[0] * grid[1]])
            .swap_dims(1, 2);
        let tokens = Tensor::cat(
            vec![self.cls_token.val().expand([batch, 1, embedding]), patches],
            1,
        );
        (tokens + self.interpolate_position(grid), grid)
    }

    fn interpolate_position(&self, [height, width]: [usize; 2]) -> Tensor<3> {
        let Dimensions {
            embedding, grid, ..
        } = self.dimensions;
        let position = self.position.val();
        if height == grid && width == grid {
            return position;
        }
        let cls = position.clone().slice([0..1, 0..1, 0..embedding]);
        let patches = position.slice([0..1, 1..1 + grid * grid, 0..embedding]);
        let device = patches.device();
        // Upstream passes scale_factor=(output + 0.1)/grid. Recomputing it from the
        // integer output size shifts patch positions and changes pretrained features.
        let rows = cubic_weights(grid, height, &device);
        let columns = cubic_weights(grid, width, &device)
            .transpose()
            .reshape([1, grid, width]);
        let patches = rows
            .matmul(patches.reshape([grid, grid * embedding]))
            .reshape([height, grid, embedding])
            .permute([0, 2, 1])
            .matmul(columns)
            .permute([0, 2, 1])
            .reshape([1, height * width, embedding]);
        Tensor::cat(vec![cls, patches], 1)
    }
}

fn norm(embedding: usize, device: &Device) -> LayerNorm {
    LayerNormConfig::new(embedding)
        .with_epsilon(1e-6)
        .init(device)
}
fn linear(input: usize, output: usize, device: &Device) -> Linear {
    let mut layer = LinearConfig::new(input, output)
        .with_initializer(Initializer::Normal {
            mean: 0.0,
            std: 0.02,
        })
        .init(device);
    layer.bias = Some(Initializer::Zeros.init([output], device));
    layer
}
fn split_features(tokens: Tensor<3>, grid: [usize; 2]) -> DinoV2Features {
    let [batch, count, embedding] = tokens.dims();
    DinoV2Features {
        cls: tokens
            .clone()
            .slice([0..batch, 0..1, 0..embedding])
            .reshape([batch, embedding]),
        patches: tokens.slice([0..batch, 1..count, 0..embedding]),
        grid,
    }
}
fn import_norm(
    layer: &mut LayerNorm,
    reader: &mut SafeTensorReader<'_>,
    name: &str,
    size: usize,
    device: &Device,
) -> Result<()> {
    layer.gamma = Param::from_tensor(reader.tensor(&format!("{name}.weight"), [size], device)?);
    layer.beta = Some(Param::from_tensor(reader.tensor(
        &format!("{name}.bias"),
        [size],
        device,
    )?));
    Ok(())
}
fn import_linear(
    layer: &mut Linear,
    reader: &mut SafeTensorReader<'_>,
    name: &str,
    input: usize,
    output: usize,
    device: &Device,
) -> Result<()> {
    layer.weight = Param::from_tensor(reader.linear_weight(
        &format!("{name}.weight"),
        input,
        output,
        device,
    )?);
    layer.bias = Some(Param::from_tensor(reader.tensor(
        &format!("{name}.bias"),
        [output],
        device,
    )?));
    Ok(())
}

fn cubic_weights(input: usize, output: usize, device: &Device) -> Tensor<2> {
    let mut weights = vec![0.0_f32; output * input];
    let scale = input as f64 / (output as f64 + 0.1);
    for destination in 0..output {
        let position = (scale * (destination as f64 + 0.5) - 0.5) as f32;
        let base = position.floor() as isize;
        for index in base - 1..=base + 2 {
            let distance = (position - index as f32).abs();
            let value = if distance <= 1.0 {
                ((1.25 * distance - 2.25) * distance) * distance + 1.0
            } else {
                ((-0.75 * distance + 3.75) * distance - 6.0) * distance + 3.0
            };
            let source = index.clamp(0, input as isize - 1) as usize;
            weights[destination * input + source] += value;
        }
    }
    Tensor::from_data(BurnData::new(weights, [output, input]), device)
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const SMALL: Dimensions = Dimensions {
        embedding: 12,
        depth: 2,
        heads: 3,
        patch: 2,
        grid: 2,
    };
    type Weights = BTreeMap<String, (Vec<usize>, Vec<f32>)>;

    fn data<const D: usize>(tensor: Tensor<D>) -> Vec<f32> {
        tensor.into_data().try_to_vec::<f32>().unwrap()
    }
    fn close(actual: &[f32], expected: &[f32], tolerance: f32) {
        assert_eq!(actual.len(), expected.len());
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (actual - expected).abs() <= tolerance,
                "At {index}: {actual} != {expected}"
            );
        }
    }
    fn input(device: &Device) -> Tensor<4> {
        Tensor::from_data(BurnData::new(input_values(), [1, 3, 4, 6]), device)
    }
    fn input_values() -> Vec<f32> {
        (0..3 * 4 * 6).map(|i| (i as f32 * 0.23).sin()).collect()
    }
    fn fixture() -> Weights {
        let mut weights = Weights::new();
        let mut add = |name: String, shape: Vec<usize>, offset: f32| {
            let seed = name.bytes().fold(0u32, |sum, value| sum + u32::from(value));
            let values = (0..shape.iter().product())
                .map(|i| offset + ((i as f32 + seed as f32) * 0.41).sin() * 0.07)
                .collect();
            weights.insert(name, (shape, values));
        };
        let e = SMALL.embedding;
        add("cls_token".into(), vec![1, 1, e], 0.0);
        add(
            "pos_embed".into(),
            vec![1, 1 + SMALL.grid * SMALL.grid, e],
            0.0,
        );
        add("mask_token".into(), vec![1, e], 0.0);
        add(
            "patch_embed.proj.weight".into(),
            vec![e, 3, SMALL.patch, SMALL.patch],
            0.0,
        );
        add("patch_embed.proj.bias".into(), vec![e], 0.0);
        for block in 0..SMALL.depth {
            let name = format!("blocks.{block}");
            for norm in ["norm1", "norm2"] {
                add(format!("{name}.{norm}.weight"), vec![e], 1.0);
                add(format!("{name}.{norm}.bias"), vec![e], 0.0);
            }
            for (layer, input, output) in [
                ("attn.qkv", e, 3 * e),
                ("attn.proj", e, e),
                ("mlp.fc1", e, 4 * e),
                ("mlp.fc2", 4 * e, e),
            ] {
                add(format!("{name}.{layer}.weight"), vec![output, input], 0.0);
                add(format!("{name}.{layer}.bias"), vec![output], 0.0);
            }
            add(format!("{name}.ls1.gamma"), vec![e], 0.8);
            add(format!("{name}.ls2.gamma"), vec![e], 0.6);
        }
        add("norm.weight".into(), vec![e], 1.0);
        add("norm.bias".into(), vec![e], 0.0);
        weights
    }
    fn serialize(weights: &Weights) -> Vec<u8> {
        let bytes = weights
            .iter()
            .map(|(name, (shape, values))| {
                (
                    name,
                    shape,
                    values
                        .iter()
                        .flat_map(|value| value.to_le_bytes())
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let views = bytes
            .iter()
            .map(|(name, shape, bytes)| {
                (
                    name.as_str(),
                    safetensors::tensor::TensorView::new(
                        safetensors::Dtype::F32,
                        (*shape).clone(),
                        bytes,
                    )
                    .unwrap(),
                )
            })
            .collect::<Vec<_>>();
        safetensors::serialize(views, None).unwrap()
    }
    fn imported(weights: &Weights, device: &Device) -> DinoV2 {
        let bytes = serialize(weights);
        let mut reader = SafeTensorReader::new(&bytes).unwrap();
        let model = DinoV2::with_dimensions(3, SMALL, device)
            .import_safetensors(&mut reader, device)
            .unwrap();
        reader.finish().unwrap();
        model
    }

    #[test]
    fn official_layout_matches_independent_scalar_reference() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, false).unwrap();
        let weights = fixture();
        let model = imported(&weights, &device);
        let features = model.features(input(&device));
        let reference = reference(&weights);
        close(&data(features.cls), &reference[..SMALL.embedding], 2e-5);
        close(&data(features.patches), &reference[SMALL.embedding..], 2e-5);
        let taps = model
            .intermediate_features(input(&device), &[0, 1], true)
            .unwrap();
        assert_eq!(taps.len(), 2);
        assert_eq!(taps[0].feature_map().dims(), [1, SMALL.embedding, 2, 3]);
        close(
            &data(taps[1].cls.clone()),
            &reference[..SMALL.embedding],
            2e-5,
        );
        assert!(
            model
                .intermediate_features(input(&device), &[1, 0], true)
                .is_err()
        );
    }

    #[test]
    fn hf_split_qkv_import_matches_meta_and_rejects_unknown_weights() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, false).unwrap();
        let meta = fixture();
        let mut hf = Weights::new();
        for (name, (shape, values)) in &meta {
            if name.contains("attn.qkv") {
                let block = name.split('.').nth(1).unwrap();
                let kind = name.rsplit('.').next().unwrap();
                let length = values.len() / 3;
                for (part, projection) in ["query", "key", "value"].iter().enumerate() {
                    let mut shape = shape.clone();
                    shape[0] /= 3;
                    hf.insert(
                        format!("encoder.layer.{block}.attention.attention.{projection}.{kind}"),
                        (shape, values[part * length..(part + 1) * length].to_vec()),
                    );
                }
                continue;
            }
            let name = match name.as_str() {
                "cls_token" => "embeddings.cls_token".into(),
                "pos_embed" => "embeddings.position_embeddings".into(),
                "mask_token" => "embeddings.mask_token".into(),
                "norm.weight" => "layernorm.weight".into(),
                "norm.bias" => "layernorm.bias".into(),
                other => other
                    .replace("patch_embed.proj", "embeddings.patch_embeddings.projection")
                    .replace("blocks.", "encoder.layer.")
                    .replace("attn.proj", "attention.output.dense")
                    .replace("ls1.gamma", "layer_scale1.lambda1")
                    .replace("ls2.gamma", "layer_scale2.lambda1"),
            };
            hf.insert(name, (shape.clone(), values.clone()));
        }
        let meta = imported(&meta, &device).features(input(&device));
        let loaded = imported(&hf, &device).features(input(&device));
        close(&data(loaded.cls), &data(meta.cls), 1e-6);
        close(&data(loaded.patches), &data(meta.patches), 1e-6);
        hf.insert(
            "register_tokens".into(),
            (vec![1, 4, SMALL.embedding], vec![0.0; 4 * SMALL.embedding]),
        );
        let bytes = serialize(&hf);
        let mut reader = SafeTensorReader::new(&bytes).unwrap();
        DinoV2::with_dimensions(3, SMALL, &device)
            .import_safetensors(&mut reader, &device)
            .unwrap();
        assert!(reader.finish().is_err());
    }

    #[test]
    fn backbone_and_head_receive_gradients_and_freezing_survives_roundtrip() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        let model = imported(&fixture(), &device).train();
        let loss = model.forward(input(&device)).powf_scalar(2.0).mean();
        let gradients = loss.backward();
        for gradient in [
            model
                .patch_embedding
                .weight
                .val()
                .grad(&gradients)
                .map(data),
            model.position.val().grad(&gradients).map(data),
            model.blocks[0]
                .attention
                .qkv
                .weight
                .val()
                .grad(&gradients)
                .map(data),
            model.classifier.weight.val().grad(&gradients).map(data),
        ] {
            let gradient = gradient.expect("Trainable backbone and head must receive gradients");
            assert!(gradient.iter().all(|value| value.is_finite()));
            assert!(gradient.iter().any(|value| value.abs() > 1e-9));
        }
        let model = model.configure_fine_tuning(true).train();
        let before = data(model.forward(input(&device)));
        let gradients = model
            .forward(input(&device))
            .powf_scalar(2.0)
            .mean()
            .backward();
        assert!(
            model
                .patch_embedding
                .weight
                .val()
                .grad(&gradients)
                .is_none()
        );
        assert!(
            model.blocks[0]
                .attention
                .qkv
                .weight
                .val()
                .grad(&gradients)
                .is_none()
        );
        assert!(model.classifier.weight.val().grad(&gradients).is_some());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dinov2.bpk");
        model.clone().save_file(&path).unwrap();
        let loaded = DinoV2::with_dimensions(3, SMALL, &device)
            .try_load_file(path)
            .unwrap()
            .configure_fine_tuning(true)
            .train();
        close(&data(loaded.forward(input(&device))), &before, 1e-6);
        let loaded = loaded
            .reset_classifier(5, &device)
            .configure_fine_tuning(false)
            .train();
        assert_eq!(loaded.forward(input(&device)).dims(), [1, 5]);
        let gradients = loaded
            .forward(input(&device))
            .powf_scalar(2.0)
            .mean()
            .backward();
        assert!(
            loaded
                .patch_embedding
                .weight
                .val()
                .grad(&gradients)
                .is_some()
        );
    }

    #[test]
    #[ignore = "requires the official facebook/dinov2-base checkpoint and normalized COCO reference image"]
    fn public_base_checkpoint_matches_upstream_reference() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let weights = std::env::var("FLOW_LIKE_DINOV2_BASE_WEIGHTS")
            .expect("Set FLOW_LIKE_DINOV2_BASE_WEIGHTS to facebook/dinov2-base/model.safetensors");
        let input = std::env::var("FLOW_LIKE_DINOV2_REFERENCE_INPUT").expect("Set FLOW_LIKE_DINOV2_REFERENCE_INPUT to the preprocessed COCO 000000039769 image as little-endian F32 NCHW");
        assert_eq!(
            crate::pretrained::file_sha256(std::path::Path::new(&weights)).unwrap(),
            "d73036b56966966d07975d696bde331762f37297e2f095de8cea0040c3aa0841"
        );
        assert_eq!(
            crate::pretrained::file_sha256(std::path::Path::new(&input)).unwrap(),
            "8d1c0a9a1561bf1d0dd72a11ac9f8762d5429c803444c51ff3e1ffe2789e8dbf"
        );
        let device = crate::backend::device(&crate::BackendChoice::Cpu, false).unwrap();
        let bytes = std::fs::read(weights).unwrap();
        let mut reader = SafeTensorReader::new(&bytes).unwrap();
        let config = DinoV2Config {
            variant: crate::DinoV2Variant::Base,
            classes: 2,
        };
        let model = DinoV2::new(&config, &device)
            .unwrap()
            .import_safetensors(&mut reader, &device)
            .unwrap();
        reader.finish().unwrap();
        let input = std::fs::read(input)
            .unwrap()
            .chunks_exact(4)
            .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
            .collect::<Vec<_>>();
        let features = model.features(Tensor::from_data(
            BurnData::new(input, [1, 3, 224, 224]),
            &device,
        ));
        assert_eq!(features.cls.dims(), [1, 768]);
        assert_eq!(features.patches.dims(), [1, 256, 768]);
        // Published integration reference, using bicubic resize to 256, center crop
        // to 224 and ImageNet normalization, from transformers v4.35.2:
        // https://github.com/huggingface/transformers/blob/v4.35.2/tests/models/dinov2/test_modeling_dinov2.py
        close(&data(features.cls)[..3], &[-2.1747, -0.4729, 1.0936], 1e-4);
        let patches = data(features.patches);
        close(&patches[..3], &[-3.2780, -0.8269, -0.9210], 1e-4);
        close(&patches[768..771], &[-2.9129, 1.1284, -0.7306], 1e-4);
    }

    fn reference(weights: &Weights) -> Vec<f32> {
        let e = SMALL.embedding;
        let image = input_values();
        let get = |name: &str| {
            weights
                .get(name)
                .unwrap()
                .1
                .iter()
                .map(|value| *value as f64)
                .collect::<Vec<_>>()
        };
        let patch_weight = get("patch_embed.proj.weight");
        let patch_bias = get("patch_embed.proj.bias");
        let position = get("pos_embed");
        let mut tokens = vec![get("cls_token")];
        for row in 0..2 {
            for column in 0..3 {
                let mut patch = patch_bias.clone();
                for output in 0..e {
                    for channel in 0..3 {
                        for y in 0..2 {
                            for x in 0..2 {
                                patch[output] +=
                                    image[(channel * 4 + row * 2 + y) * 6 + column * 2 + x] as f64
                                        * patch_weight[((output * 3 + channel) * 2 + y) * 2 + x];
                            }
                        }
                    }
                }
                tokens.push(patch);
            }
        }
        for channel in 0..e {
            tokens[0][channel] += position[channel];
        }
        // A scalar 16-tap bicubic reference, independent of the separable tensor path.
        let kernel = |distance: f64| {
            let x = distance.abs();
            if x <= 1.0 {
                1.25 * x.powi(3) - 2.25 * x.powi(2) + 1.0
            } else if x < 2.0 {
                -0.75 * x.powi(3) + 3.75 * x.powi(2) - 6.0 * x + 3.0
            } else {
                0.0
            }
        };
        for row in 0..2 {
            for column in 0..3 {
                let y = 2.0 * (row as f64 + 0.5) / 2.1 - 0.5;
                let x = 2.0 * (column as f64 + 0.5) / 3.1 - 0.5;
                for sy in y.floor() as isize - 1..=y.floor() as isize + 2 {
                    for sx in x.floor() as isize - 1..=x.floor() as isize + 2 {
                        let factor = kernel(y - sy as f64) * kernel(x - sx as f64);
                        let source = 1 + sy.clamp(0, 1) as usize * 2 + sx.clamp(0, 1) as usize;
                        for channel in 0..e {
                            tokens[1 + row * 3 + column][channel] +=
                                factor * position[source * e + channel];
                        }
                    }
                }
            }
        }
        let normalized = |x: &[f64], name: &str| {
            let gamma = get(&format!("{name}.weight"));
            let beta = get(&format!("{name}.bias"));
            let mean = x.iter().sum::<f64>() / x.len() as f64;
            let variance = x.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / x.len() as f64;
            x.iter()
                .enumerate()
                .map(|(i, v)| (v - mean) / (variance + 1e-6).sqrt() * gamma[i] + beta[i])
                .collect::<Vec<_>>()
        };
        let affine = |x: &[f64], name: &str| {
            let weight = get(&format!("{name}.weight"));
            let mut output = get(&format!("{name}.bias"));
            for (o, value) in output.iter_mut().enumerate() {
                *value += x
                    .iter()
                    .enumerate()
                    .map(|(i, value)| value * weight[o * x.len() + i])
                    .sum::<f64>();
            }
            output
        };
        for block in 0..SMALL.depth {
            let prefix = format!("blocks.{block}");
            let qkv = tokens
                .iter()
                .map(|token| {
                    affine(
                        &normalized(token, &format!("{prefix}.norm1")),
                        &format!("{prefix}.attn.qkv"),
                    )
                })
                .collect::<Vec<_>>();
            let scale1 = get(&format!("{prefix}.ls1.gamma"));
            let scale2 = get(&format!("{prefix}.ls2.gamma"));
            for (index, token) in tokens.iter_mut().enumerate() {
                let mut mixed = vec![0.0; e];
                let size = e / SMALL.heads;
                for head in 0..SMALL.heads {
                    let scores = qkv
                        .iter()
                        .map(|kv| {
                            (0..size)
                                .map(|d| qkv[index][head * size + d] * kv[e + head * size + d])
                                .sum::<f64>()
                                / (size as f64).sqrt()
                        })
                        .collect::<Vec<_>>();
                    let maximum = scores.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    let total = scores
                        .iter()
                        .map(|value| (value - maximum).exp())
                        .sum::<f64>();
                    for (key, score) in scores.iter().enumerate() {
                        let probability = (score - maximum).exp() / total;
                        for d in 0..size {
                            mixed[head * size + d] +=
                                probability * qkv[key][2 * e + head * size + d];
                        }
                    }
                }
                let attended = affine(&mixed, &format!("{prefix}.attn.proj"));
                for d in 0..e {
                    token[d] += attended[d] * scale1[d];
                }
                let hidden = affine(
                    &normalized(token, &format!("{prefix}.norm2")),
                    &format!("{prefix}.mlp.fc1"),
                );
                let hidden = hidden.into_iter().map(reference_gelu).collect::<Vec<_>>();
                let hidden = affine(&hidden, &format!("{prefix}.mlp.fc2"));
                for d in 0..e {
                    token[d] += hidden[d] * scale2[d];
                }
            }
        }
        tokens
            .iter()
            .flat_map(|token| normalized(token, "norm"))
            .map(|v| v as f32)
            .collect()
    }
    fn reference_gelu(value: f64) -> f64 {
        // Simpson integration evaluates the normal CDF without sharing Burn's GELU primitive.
        let steps = 128;
        let step = value / steps as f64;
        let mut integral = 0.0;
        for i in 0..=steps {
            let coefficient = if i == 0 || i == steps {
                1.0
            } else if i % 2 == 0 {
                2.0
            } else {
                4.0
            };
            integral += coefficient * (-0.5 * (i as f64 * step).powi(2)).exp();
        }
        value * (0.5 + integral * step / (3.0 * (2.0 * std::f64::consts::PI).sqrt()))
    }
}

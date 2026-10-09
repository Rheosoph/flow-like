// Native adaptation of RF-DETR, Copyright (c) 2025 Roboflow.
// Upstream backbone: Copyright 2022-2024 HuggingFace and Meta.
// Licensed under Apache-2.0; see licenses/RF-DETR and licenses/DINOv2.
use crate::{Result, dinov2::Block, pretrained::SafeTensorReader};
use burn::{
    module::{Module, Param},
    nn::{
        Initializer, LayerNorm, LayerNormConfig, PaddingConfig2d,
        conv::{Conv2d, Conv2dConfig},
    },
    tensor::{Device, Tensor, TensorData, activation::silu},
};

#[derive(Clone, Debug)]
pub(crate) struct BackboneDimensions {
    pub embedding: usize,
    pub depth: usize,
    pub heads: usize,
    pub patch: usize,
    pub grid: usize,
    pub windows: usize,
    pub stages: Vec<usize>,
}

#[derive(Module, Debug)]
pub(crate) struct RfBackbone {
    patch: Conv2d,
    cls: Param<Tensor<3>>,
    mask: Param<Tensor<2>>,
    position: Param<Tensor<3>>,
    blocks: Vec<Block>,
    norm: LayerNorm,
    #[module(skip)]
    dimensions: BackboneDimensions,
}
impl RfBackbone {
    pub(crate) fn new(dimensions: BackboneDimensions, device: &Device) -> Self {
        let d = &dimensions;
        Self {
            patch: Conv2dConfig::new([3, d.embedding], [d.patch, d.patch])
                .with_stride([d.patch, d.patch])
                .init(device),
            cls: Initializer::Normal {
                mean: 0.0,
                std: 0.02,
            }
            .init([1, 1, d.embedding], device),
            mask: Initializer::Zeros.init([1, d.embedding], device),
            position: Initializer::Normal {
                mean: 0.0,
                std: 0.02,
            }
            .init([1, 1 + d.grid * d.grid, d.embedding], device),
            blocks: (0..d.depth)
                .map(|_| Block::new(d.embedding, d.heads, device))
                .collect(),
            norm: channel_norm(d.embedding, device),
            dimensions,
        }
    }
    pub(crate) fn forward(&self, input: Tensor<4>) -> Vec<Tensor<4>> {
        let [batch, _, height, width] = input.dims();
        let d = &self.dimensions;
        let rows = height / d.patch;
        let columns = width / d.patch;
        let window_rows = rows / d.windows;
        let window_columns = columns / d.windows;
        let count = d.windows * d.windows;
        let patches = self
            .patch
            .forward(input)
            .reshape([batch, d.embedding, rows * columns])
            .swap_dims(1, 2);
        let tokens = Tensor::cat(
            vec![self.cls.val().expand([batch, 1, d.embedding]), patches],
            1,
        ) + self.position([rows, columns]);
        let cls = tokens
            .clone()
            .slice([0..batch, 0..1, 0..d.embedding])
            .reshape([batch, 1, 1, d.embedding])
            .expand([batch, count, 1, d.embedding])
            .reshape([batch * count, 1, d.embedding]);
        let patches = tokens
            .slice([0..batch, 1..1 + rows * columns, 0..d.embedding])
            .reshape([
                batch * d.windows,
                window_rows,
                d.windows,
                window_columns,
                d.embedding,
            ])
            .permute([0, 2, 1, 3, 4])
            .reshape([batch * count, window_rows * window_columns, d.embedding]);
        let mut tokens = Tensor::cat(vec![cls, patches], 1);
        let window_tokens = 1 + window_rows * window_columns;
        let mut features = Vec::with_capacity(d.stages.len());
        for (index, block) in self.blocks.iter().enumerate() {
            // Released checkpoints derive global block indices from the one-based
            // feature-stage list without subtracting one. Preserve that routing.
            let global = d.stages.contains(&index);
            tokens = if global {
                block
                    .forward(tokens.reshape([batch, count * window_tokens, d.embedding]))
                    .reshape([batch * count, window_tokens, d.embedding])
            } else {
                block.forward(tokens)
            };
            if d.stages.contains(&(index + 1)) {
                let output = self.norm.forward(tokens.clone()).slice([
                    0..batch * count,
                    1..window_tokens,
                    0..d.embedding,
                ]);
                let output = output
                    .reshape([
                        batch * d.windows,
                        d.windows,
                        window_rows,
                        window_columns,
                        d.embedding,
                    ])
                    .permute([0, 2, 1, 3, 4])
                    .reshape([batch, rows, columns, d.embedding])
                    .permute([0, 3, 1, 2]);
                features.push(output);
            }
            if features.len() == d.stages.len() {
                break;
            }
        }
        features
    }
    pub(crate) fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        let d = &self.dimensions;
        self.patch = reader.conv2d(
            self.patch,
            &format!("{prefix}.embeddings.patch_embeddings.projection"),
            device,
        )?;
        self.cls = Param::from_tensor(reader.tensor(
            &format!("{prefix}.embeddings.cls_token"),
            [1, 1, d.embedding],
            device,
        )?);
        self.mask = Param::from_tensor(reader.tensor(
            &format!("{prefix}.embeddings.mask_token"),
            [1, d.embedding],
            device,
        )?);
        self.position = Param::from_tensor(reader.tensor(
            &format!("{prefix}.embeddings.position_embeddings"),
            [1, 1 + d.grid * d.grid, d.embedding],
            device,
        )?);
        for (index, block) in self.blocks.iter_mut().enumerate() {
            block.import_hugging_face(
                reader,
                &format!("{prefix}.encoder.layer.{index}"),
                device,
            )?;
        }
        self.norm = reader.layer_norm(self.norm, &format!("{prefix}.layernorm"), device)?;
        Ok(self)
    }
    fn position(&self, [rows, columns]: [usize; 2]) -> Tensor<3> {
        let d = &self.dimensions;
        let position = self.position.val();
        if rows == d.grid && columns == d.grid {
            return position;
        }
        let cls = position.clone().slice([0..1, 0..1, 0..d.embedding]);
        let patches = position.slice([0..1, 1..1 + d.grid * d.grid, 0..d.embedding]);
        let device = patches.device();
        let wy = antialiased_weights(d.grid, rows, &device);
        let wx = antialiased_weights(d.grid, columns, &device)
            .transpose()
            .reshape([1, d.grid, columns]);
        let patches = wy
            .matmul(patches.reshape([d.grid, d.grid * d.embedding]))
            .reshape([rows, d.grid, d.embedding])
            .permute([0, 2, 1])
            .matmul(wx)
            .permute([0, 2, 1])
            .reshape([1, rows * columns, d.embedding]);
        Tensor::cat(vec![cls, patches], 1)
    }
}

fn channel_norm(channels: usize, device: &Device) -> LayerNorm {
    LayerNormConfig::new(channels)
        .with_epsilon(1e-6)
        .init(device)
}

#[derive(Module, Debug)]
struct ConvNorm {
    conv: Conv2d,
    norm: LayerNorm,
}
impl ConvNorm {
    fn new(input: usize, output: usize, kernel: usize, device: &Device) -> Self {
        let pad = kernel / 2;
        Self {
            conv: Conv2dConfig::new([input, output], [kernel, kernel])
                .with_padding(PaddingConfig2d::Explicit(pad, pad, pad, pad))
                .with_bias(false)
                .init(device),
            norm: channel_norm(output, device),
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        silu(
            self.norm
                .forward(self.conv.forward(input).permute([0, 2, 3, 1]))
                .permute([0, 3, 1, 2]),
        )
    }
    fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.conv = reader.conv2d(self.conv, &format!("{prefix}.conv"), device)?;
        self.norm = reader.layer_norm(self.norm, &format!("{prefix}.bn"), device)?;
        Ok(self)
    }
}
#[derive(Module, Debug)]
struct Bottleneck {
    first: ConvNorm,
    second: ConvNorm,
}
impl Bottleneck {
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        self.second.forward(self.first.forward(input))
    }
}
#[derive(Module, Debug)]
pub(crate) struct Projector {
    first: ConvNorm,
    second: ConvNorm,
    blocks: Vec<Bottleneck>,
    norm: LayerNorm,
    hidden: usize,
}
impl Projector {
    pub(crate) fn new(input: usize, output: usize, device: &Device) -> Self {
        let hidden = output / 2;
        Self {
            first: ConvNorm::new(input, 2 * hidden, 1, device),
            second: ConvNorm::new(5 * hidden, output, 1, device),
            blocks: (0..3)
                .map(|_| Bottleneck {
                    first: ConvNorm::new(hidden, hidden, 3, device),
                    second: ConvNorm::new(hidden, hidden, 3, device),
                })
                .collect(),
            norm: channel_norm(output, device),
            hidden,
        }
    }
    pub(crate) fn forward(&self, inputs: Vec<Tensor<4>>) -> Tensor<4> {
        let features = self.first.forward(Tensor::cat(inputs, 1));
        let [batch, _, height, width] = features.dims();
        let mut parts = vec![
            features
                .clone()
                .slice([0..batch, 0..self.hidden, 0..height, 0..width]),
            features.slice([0..batch, self.hidden..2 * self.hidden, 0..height, 0..width]),
        ];
        for block in &self.blocks {
            parts.push(block.forward(parts.last().unwrap().clone()));
        }
        self.norm
            .forward(
                self.second
                    .forward(Tensor::cat(parts, 1))
                    .permute([0, 2, 3, 1]),
            )
            .permute([0, 3, 1, 2])
    }
    pub(crate) fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.first = self
            .first
            .import(&format!("{prefix}.stages.0.0.cv1"), reader, device)?;
        self.second = self
            .second
            .import(&format!("{prefix}.stages.0.0.cv2"), reader, device)?;
        self.blocks = self
            .blocks
            .into_iter()
            .enumerate()
            .map(|(index, block)| {
                Ok(Bottleneck {
                    first: block.first.import(
                        &format!("{prefix}.stages.0.0.m.{index}.cv1"),
                        reader,
                        device,
                    )?,
                    second: block.second.import(
                        &format!("{prefix}.stages.0.0.m.{index}.cv2"),
                        reader,
                        device,
                    )?,
                })
            })
            .collect::<Result<_>>()?;
        self.norm = reader.layer_norm(self.norm, &format!("{prefix}.stages.0.1"), device)?;
        Ok(self)
    }
}

// PyTorch's antialiased bicubic path uses the Pillow cubic kernel and normalizes
// the retained taps at image boundaries, including when the grid is enlarged.
fn antialiased_weights(input: usize, output: usize, device: &Device) -> Tensor<2> {
    let scale = input as f64 / output as f64;
    let filter_scale = scale.max(1.0);
    let support = 2.0 * filter_scale;
    let mut weights = vec![0.0_f32; output * input];
    for destination in 0..output {
        let center = (destination as f64 + 0.5) * scale;
        let first = ((center - support + 0.5) as isize).max(0) as usize;
        let last = ((center + support + 0.5) as usize).min(input);
        let mut total = 0.0;
        for source in first..last {
            let x = ((source as f64 + 0.5 - center) / filter_scale).abs();
            let value = if x < 1.0 {
                1.5 * x.powi(3) - 2.5 * x.powi(2) + 1.0
            } else if x < 2.0 {
                -0.5 * x.powi(3) + 2.5 * x.powi(2) - 4.0 * x + 2.0
            } else {
                0.0
            } as f32;
            weights[destination * input + source] = value;
            total += value;
        }
        for source in first..last {
            weights[destination * input + source] /= total;
        }
    }
    Tensor::from_data(TensorData::new(weights, [output, input]), device)
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;

    #[test]
    fn antialiased_positions_preserve_constants_and_boundary_weights() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        for (input, output) in [(37, 40), (37, 24), (2, 3)] {
            let weights = antialiased_weights(input, output, &device);
            for sum in weights.clone().sum_dim(1).into_data().iter::<f32>() {
                assert!((sum - 1.0).abs() < 2e-6);
            }
            let positions = Tensor::<2>::ones([input, 3], &device).require_grad();
            let output = weights.matmul(positions.clone());
            for value in output.clone().into_data().iter::<f32>() {
                assert!((value - 1.0).abs() < 2e-6);
            }
            let grad = positions.grad(&output.sum().backward()).unwrap();
            assert!(grad.into_data().iter::<f32>().all(|v| v.is_finite()));
        }
        let weights = antialiased_weights(2, 3, &device)
            .into_data()
            .try_to_vec::<f32>()
            .unwrap();
        assert!(
            weights[0] > 1.0 && weights[1] < 0.0,
            "Bicubic edge overshoot must not become bilinear clipping"
        );
        assert!((weights[2] - 0.5).abs() < 1e-6 && (weights[3] - 0.5).abs() < 1e-6);
    }
}

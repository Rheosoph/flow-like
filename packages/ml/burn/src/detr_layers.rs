// Native port of RT-DETR transformer components, Copyright (c) 2023 lyuwenyu.
// Upstream source is licensed under Apache-2.0; see licenses/RT-DETR.
use crate::{Result, pretrained::SafeTensorReader};
use burn::{
    module::{Module, Param},
    nn::{
        BatchNorm, BatchNormConfig, Initializer, LayerNorm, LayerNormConfig, Linear, LinearConfig,
        PaddingConfig2d,
        conv::{Conv2d, Conv2dConfig},
    },
    tensor::{
        Device, Tensor, TensorData,
        activation::{relu, softmax},
        ops::GridSampleOptions,
    },
};

pub(crate) fn linear(input: usize, output: usize, device: &Device) -> Linear {
    LinearConfig::new(input, output)
        .with_initializer(Initializer::XavierUniform { gain: 1.0 })
        .init(device)
}
pub(crate) fn norm(channels: usize, device: &Device) -> LayerNorm {
    LayerNormConfig::new(channels)
        .with_epsilon(1e-5)
        .init(device)
}

#[derive(Module, Debug)]
pub(crate) struct ConvNorm {
    pub(crate) conv: Conv2d,
    pub(crate) norm: BatchNorm,
}
impl ConvNorm {
    pub(crate) fn new(
        input: usize,
        output: usize,
        kernel: usize,
        stride: usize,
        device: &Device,
    ) -> Self {
        let pad = kernel / 2;
        Self {
            conv: Conv2dConfig::new([input, output], [kernel, kernel])
                .with_stride([stride, stride])
                .with_bias(false)
                .with_padding(PaddingConfig2d::Explicit(pad, pad, pad, pad))
                .init(device),
            norm: BatchNormConfig::new(output)
                .with_epsilon(1e-5)
                .with_momentum(0.1)
                .init(device),
        }
    }
    pub(crate) fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        self.norm.forward(self.conv.forward(input))
    }
    pub(crate) fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.conv = reader.conv2d(self.conv, &format!("{prefix}.conv"), device)?;
        self.norm = reader.batch_norm(self.norm, &format!("{prefix}.norm"), device)?;
        Ok(self)
    }
}

#[derive(Module, Debug)]
pub(crate) struct Mlp {
    pub(crate) layers: Vec<Linear>,
}
impl Mlp {
    pub(crate) fn new(
        input: usize,
        hidden: usize,
        output: usize,
        depth: usize,
        device: &Device,
    ) -> Self {
        Self {
            layers: (0..depth)
                .map(|i| {
                    linear(
                        if i == 0 { input } else { hidden },
                        if i + 1 == depth { output } else { hidden },
                        device,
                    )
                })
                .collect(),
        }
    }
    pub(crate) fn zero_last(mut self) -> Self {
        let last = self.layers.last_mut().unwrap();
        last.weight = last.weight.clone().map(|w| w.zeros_like());
        last.bias = last.bias.take().map(|b| b.map(|v| v.zeros_like()));
        self
    }
    pub(crate) fn forward(&self, mut input: Tensor<3>) -> Tensor<3> {
        for (i, layer) in self.layers.iter().enumerate() {
            input = layer.forward(input);
            if i + 1 < self.layers.len() {
                input = relu(input);
            }
        }
        input
    }
    pub(crate) fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.layers = self
            .layers
            .into_iter()
            .enumerate()
            .map(|(i, layer)| reader.linear(layer, &format!("{prefix}.layers.{i}"), device))
            .collect::<Result<_>>()?;
        Ok(self)
    }
}

#[derive(Module, Debug)]
pub(crate) struct Attention {
    q: Linear,
    k: Linear,
    v: Linear,
    out: Linear,
    heads: usize,
}
impl Attention {
    pub(crate) fn new(channels: usize, heads: usize, device: &Device) -> Self {
        Self {
            q: linear(channels, channels, device),
            k: linear(channels, channels, device),
            v: linear(channels, channels, device),
            out: linear(channels, channels, device),
            heads,
        }
    }
    pub(crate) fn forward(&self, query: Tensor<3>, key: Tensor<3>, value: Tensor<3>) -> Tensor<3> {
        let [batch, queries, channels] = query.dims();
        let tokens = key.dims()[1];
        let dim = channels / self.heads;
        let q = self
            .q
            .forward(query)
            .reshape([batch, queries, self.heads, dim])
            .swap_dims(1, 2);
        let k = self
            .k
            .forward(key)
            .reshape([batch, tokens, self.heads, dim])
            .permute([0, 2, 3, 1]);
        let v = self
            .v
            .forward(value)
            .reshape([batch, tokens, self.heads, dim])
            .swap_dims(1, 2);
        let output = softmax(q.matmul(k) / (dim as f64).sqrt(), 3).matmul(v);
        self.out
            .forward(output.swap_dims(1, 2).reshape([batch, queries, channels]))
    }
    pub(crate) fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        let channels = self.q.weight.val().dims()[0];
        let weight = reader.tensor::<2>(
            &format!("{prefix}.in_proj_weight"),
            [3 * channels, channels],
            device,
        )?;
        let bias = reader.tensor::<1>(&format!("{prefix}.in_proj_bias"), [3 * channels], device)?;
        for (i, layer) in [&mut self.q, &mut self.k, &mut self.v]
            .into_iter()
            .enumerate()
        {
            layer.weight = Param::from_tensor(
                weight
                    .clone()
                    .slice([i * channels..(i + 1) * channels, 0..channels])
                    .transpose(),
            );
            layer.bias = Some(Param::from_tensor(
                bias.clone().slice([i * channels..(i + 1) * channels]),
            ));
        }
        self.out = reader.linear(self.out, &format!("{prefix}.out_proj"), device)?;
        Ok(self)
    }
}

/// Samples a small set of learned locations per query and pyramid level.
#[derive(Module, Debug)]
pub(crate) struct DeformableAttention {
    offsets: Linear,
    attention: Linear,
    value: Linear,
    output: Linear,
    heads: usize,
    #[module(skip)]
    points: Vec<usize>,
}
impl DeformableAttention {
    pub(crate) fn new(channels: usize, heads: usize, points: Vec<usize>, device: &Device) -> Self {
        let count = points.iter().sum::<usize>();
        let zero_linear = |output| {
            LinearConfig::new(channels, output)
                .with_initializer(Initializer::Zeros)
                .init(device)
        };
        let mut offsets = zero_linear(heads * count * 2);
        let mut bias = Vec::with_capacity(heads * count * 2);
        for head in 0..heads {
            let angle = head as f32 * std::f32::consts::TAU / heads as f32;
            let (y, x) = angle.sin_cos();
            let scale = x.abs().max(y.abs());
            for &number in &points {
                for point in 1..=number {
                    bias.extend([x / scale * point as f32, y / scale * point as f32]);
                }
            }
        }
        offsets.bias = Some(Param::from_tensor(Tensor::from_data(
            TensorData::new(bias, [heads * count * 2]),
            device,
        )));
        Self {
            offsets,
            attention: zero_linear(heads * count),
            value: linear(channels, channels, device),
            output: linear(channels, channels, device),
            heads,
            points,
        }
    }
    pub(crate) fn forward(
        &self,
        query: Tensor<3>,
        references: Tensor<3>,
        memory: Tensor<3>,
        shapes: &[[usize; 2]],
    ) -> Tensor<3> {
        let [batch, queries, channels] = query.dims();
        let tokens = memory.dims()[1];
        let head_dim = channels / self.heads;
        let count = self.points.iter().sum::<usize>();
        let values = self
            .value
            .forward(memory)
            .reshape([batch, tokens, self.heads, head_dim]);
        let offsets = self
            .offsets
            .forward(query.clone())
            .reshape([batch, queries, self.heads, count, 2]);
        let weights = softmax(
            self.attention
                .forward(query)
                .reshape([batch, queries, self.heads, count]),
            3,
        )
        .permute([0, 2, 1, 3])
        .reshape([batch * self.heads, 1, queries, count]);
        let xy = references
            .clone()
            .slice([0..batch, 0..queries, 0..2])
            .reshape([batch, queries, 1, 1, 2]);
        let wh = references
            .slice([0..batch, 0..queries, 2..4])
            .reshape([batch, queries, 1, 1, 2]);
        let mut sampled = Vec::with_capacity(shapes.len());
        let (mut token_start, mut point_start) = (0, 0);
        for (level, &[height, width]) in shapes.iter().enumerate() {
            let points = self.points[level];
            let grid = xy.clone()
                + offsets.clone().slice([
                    0..batch,
                    0..queries,
                    0..self.heads,
                    point_start..point_start + points,
                    0..2,
                ]) * wh.clone()
                    * (0.5 / points as f64);
            let grid = (grid * 2.0 - 1.0).permute([0, 2, 1, 3, 4]).reshape([
                batch * self.heads,
                queries,
                points,
                2,
            ]);
            let value = values
                .clone()
                .slice([
                    0..batch,
                    token_start..token_start + height * width,
                    0..self.heads,
                    0..head_dim,
                ])
                .permute([0, 2, 3, 1])
                .reshape([batch * self.heads, head_dim, height, width]);
            sampled.push(value.grid_sample_2d(grid, GridSampleOptions::default()));
            token_start += height * width;
            point_start += points;
        }
        let result = (Tensor::cat(sampled, 3) * weights)
            .sum_dim(3)
            .reshape([batch, self.heads * head_dim, queries])
            .swap_dims(1, 2);
        self.output.forward(result)
    }
    pub(crate) fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self = self.import_without_point_scale(prefix, reader, device)?;
        let count = self.points.iter().sum::<usize>();
        // RT-DETRv2 persists this derived buffer. Validate it instead of trusting it.
        let buffer = reader.tensor::<1>(&format!("{prefix}.num_points_scale"), [count], device)?;
        let actual = buffer
            .to_data()
            .try_to_vec::<f32>()
            .map_err(|e| crate::Error::Invalid(format!("Invalid point scale: {e}")))?;
        let expected: Vec<f32> = self
            .points
            .iter()
            .flat_map(|&n| vec![1.0 / n as f32; n])
            .collect();
        if actual != expected {
            return Err(crate::Error::Invalid(
                "Checkpoint point scales do not match the detector configuration".into(),
            ));
        }
        Ok(self)
    }
    pub(crate) fn import_without_point_scale(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.offsets =
            reader.linear(self.offsets, &format!("{prefix}.sampling_offsets"), device)?;
        self.attention = reader.linear(
            self.attention,
            &format!("{prefix}.attention_weights"),
            device,
        )?;
        self.value = reader.linear(self.value, &format!("{prefix}.value_proj"), device)?;
        self.output = reader.linear(self.output, &format!("{prefix}.output_proj"), device)?;
        Ok(self)
    }
}

#[derive(Module, Debug)]
pub(crate) struct DecoderLayer {
    attention: Attention,
    cross: DeformableAttention,
    first: Linear,
    second: Linear,
    norm1: LayerNorm,
    norm2: LayerNorm,
    norm3: LayerNorm,
}
impl DecoderLayer {
    pub(crate) fn new(
        channels: usize,
        heads: usize,
        points: Vec<usize>,
        feedforward: usize,
        device: &Device,
    ) -> Self {
        Self {
            attention: Attention::new(channels, heads, device),
            cross: DeformableAttention::new(channels, heads, points, device),
            first: linear(channels, feedforward, device),
            second: linear(feedforward, channels, device),
            norm1: norm(channels, device),
            norm2: norm(channels, device),
            norm3: norm(channels, device),
        }
    }
    pub(crate) fn forward(
        &self,
        target: Tensor<3>,
        position: Tensor<3>,
        references: Tensor<3>,
        memory: Tensor<3>,
        shapes: &[[usize; 2]],
    ) -> Tensor<3> {
        let positioned = target.clone() + position.clone();
        let target = self.norm1.forward(
            target.clone()
                + self
                    .attention
                    .forward(positioned.clone(), positioned, target),
        );
        let target = self.norm2.forward(
            target.clone()
                + self
                    .cross
                    .forward(target + position, references, memory, shapes),
        );
        self.norm3
            .forward(target.clone() + self.second.forward(relu(self.first.forward(target))))
    }
    pub(crate) fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.attention = self
            .attention
            .import(&format!("{prefix}.self_attn"), reader, device)?;
        self.cross = self
            .cross
            .import(&format!("{prefix}.cross_attn"), reader, device)?;
        self.first = reader.linear(self.first, &format!("{prefix}.linear1"), device)?;
        self.second = reader.linear(self.second, &format!("{prefix}.linear2"), device)?;
        self.norm1 = reader.layer_norm(self.norm1, &format!("{prefix}.norm1"), device)?;
        self.norm2 = reader.layer_norm(self.norm2, &format!("{prefix}.norm2"), device)?;
        self.norm3 = reader.layer_norm(self.norm3, &format!("{prefix}.norm3"), device)?;
        Ok(self)
    }
}

pub(crate) fn inverse_sigmoid<const D: usize>(input: Tensor<D>) -> Tensor<D> {
    let input = input.clamp(0.0, 1.0);
    (input.clone().clamp_min(1e-5) / (input.neg() + 1.0).clamp_min(1e-5)).log()
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;

    #[test]
    fn deformable_attention_samples_levels_and_heads_with_native_gradients() {
        let _guard = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let device = crate::backend::device(&crate::BackendChoice::Cpu, true).unwrap();
        let mut layer = DeformableAttention::new(4, 2, vec![1, 1], &device);
        layer.offsets.bias = Some(Param::from_tensor(Tensor::zeros([8], &device)));
        let identity = || {
            Tensor::from_data(
                [
                    [1.0f32, 0., 0., 0.],
                    [0., 1., 0., 0.],
                    [0., 0., 1., 0.],
                    [0., 0., 0., 1.],
                ],
                &device,
            )
        };
        for projection in [&mut layer.value, &mut layer.output] {
            projection.weight = Param::from_tensor(identity());
            projection.bias = Some(Param::from_tensor(Tensor::zeros([4], &device)));
        }
        let values = Tensor::<3>::from_data(
            TensorData::new((0..20).map(|n| n as f32).collect::<Vec<_>>(), [1, 5, 4]),
            &device,
        )
        .require_grad();
        let output = layer.forward(
            Tensor::zeros([1, 1, 4], &device),
            Tensor::from_data([[[0.5f32, 0.5, 0.5, 0.5]]], &device),
            values.clone(),
            &[[2, 2], [1, 1]],
        );
        let actual = output.to_data().try_to_vec::<f32>().unwrap();
        for (a, b) in actual.iter().zip([11.0, 12.0, 13.0, 14.0]) {
            assert!((a - b).abs() < 1e-6, "{actual:?}");
        }
        let gradients = output.sum().backward();
        let gradient = values
            .grad(&gradients)
            .unwrap()
            .to_data()
            .try_to_vec::<f32>()
            .unwrap();
        for (index, value) in gradient.into_iter().enumerate() {
            let expected = if index < 16 { 0.125 } else { 0.5 };
            assert!(
                (value - expected).abs() < 1e-6,
                "input {index}: {value} != {expected}"
            );
        }
    }
}

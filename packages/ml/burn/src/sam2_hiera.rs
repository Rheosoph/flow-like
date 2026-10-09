// Copyright (c) Meta Platforms, Inc. and affiliates. All rights reserved.
// Adapted from SAM 2.1 Hiera-T under Apache-2.0; see licenses/SAM2.
use crate::{Result, pretrained::SafeTensorReader};
use burn::{
    module::{Module, Param},
    nn::{
        Initializer, LayerNorm, LayerNormConfig, Linear, LinearConfig, PaddingConfig2d,
        conv::{Conv2d, Conv2dConfig},
        pool::MaxPool2dConfig,
    },
    tensor::{
        Device, Tensor,
        activation::{gelu, softmax},
        module::interpolate,
        ops::{InterpolateMode, InterpolateOptions, PadMode},
    },
};

pub(crate) fn resize(input: Tensor<4>, size: [usize; 2], mode: InterpolateMode) -> Tensor<4> {
    interpolate(
        input,
        InterpolateOptions::new(mode)
            .with_output_size(size)
            .with_align_corners(false),
    )
}
fn norm(channels: usize, device: &Device) -> LayerNorm {
    LayerNormConfig::new(channels)
        .with_epsilon(1e-6)
        .init(device)
}
fn pool(input: Tensor<4>) -> Tensor<4> {
    MaxPool2dConfig::new([2, 2])
        .with_strides([2, 2])
        .init()
        .forward(input.permute([0, 3, 1, 2]))
        .permute([0, 2, 3, 1])
}

#[derive(Module, Debug)]
struct HieraBlock {
    norm1: LayerNorm,
    qkv: Linear,
    attention_projection: Linear,
    projection: Option<Linear>,
    norm2: LayerNorm,
    mlp1: Linear,
    mlp2: Linear,
    heads: usize,
    downsample: bool,
    window: usize,
}
impl HieraBlock {
    fn new(
        input: usize,
        output: usize,
        heads: usize,
        window: usize,
        downsample: bool,
        device: &Device,
    ) -> Self {
        Self {
            norm1: norm(input, device),
            qkv: LinearConfig::new(input, output * 3).init(device),
            attention_projection: LinearConfig::new(output, output).init(device),
            projection: (input != output).then(|| LinearConfig::new(input, output).init(device)),
            norm2: norm(output, device),
            mlp1: LinearConfig::new(output, output * 4).init(device),
            mlp2: LinearConfig::new(output * 4, output).init(device),
            heads,
            downsample,
            window,
        }
    }
    fn attention(&self, input: Tensor<4>) -> Tensor<4> {
        let [batch, height, width, _] = input.dims();
        let output = self.attention_projection.weight.dims()[0];
        let channels = output / self.heads;
        let qkv = self
            .qkv
            .forward(input)
            .reshape([batch, height * width, 3, output]);
        let part = |index| {
            qkv.clone()
                .slice([0..batch, 0..height * width, index..index + 1, 0..output])
                .reshape([batch, height, width, output])
        };
        let query = if self.downsample {
            pool(part(0))
        } else {
            part(0)
        };
        let [_, q_height, q_width, _] = query.dims();
        let query = query
            .reshape([batch, q_height * q_width, self.heads, channels])
            .permute([0, 2, 1, 3]);
        let key = part(1)
            .reshape([batch, height * width, self.heads, channels])
            .permute([0, 2, 3, 1]);
        let value = part(2)
            .reshape([batch, height * width, self.heads, channels])
            .permute([0, 2, 1, 3]);
        self.attention_projection.forward(
            softmax(query.matmul(key) / (channels as f64).sqrt(), 3)
                .matmul(value)
                .permute([0, 2, 1, 3])
                .reshape([batch, q_height, q_width, output]),
        )
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let normalized = self.norm1.forward(input.clone());
        let shortcut = match &self.projection {
            Some(projection) => pool(projection.forward(normalized.clone())),
            None => input,
        };
        let mut output = if self.window == 0 {
            self.attention(normalized)
        } else {
            let [batch, height, width, channels] = normalized.dims();
            let ws = self.window;
            let padded_h = height.div_ceil(ws) * ws;
            let padded_w = width.div_ceil(ws) * ws;
            let padded = normalized.pad(
                [
                    (0, 0),
                    (0, padded_h - height),
                    (0, padded_w - width),
                    (0, 0),
                ],
                PadMode::Constant(0.0),
            );
            let windows = padded
                .reshape([batch, padded_h / ws, ws, padded_w / ws, ws, channels])
                .permute([0, 1, 3, 2, 4, 5])
                .reshape([batch * (padded_h / ws) * (padded_w / ws), ws, ws, channels]);
            let attended = self.attention(windows);
            let window_out = if self.downsample { ws / 2 } else { ws };
            let [_, out_h, out_w, out_c] = shortcut.dims();
            let padded_out_h = out_h.div_ceil(window_out) * window_out;
            let padded_out_w = out_w.div_ceil(window_out) * window_out;
            attended
                .reshape([
                    batch,
                    padded_out_h / window_out,
                    padded_out_w / window_out,
                    window_out,
                    window_out,
                    out_c,
                ])
                .permute([0, 1, 3, 2, 4, 5])
                .reshape([batch, padded_out_h, padded_out_w, out_c])
                .slice([0..batch, 0..out_h, 0..out_w, 0..out_c])
        };
        output = shortcut + output;
        output.clone()
            + self
                .mlp2
                .forward(gelu(self.mlp1.forward(self.norm2.forward(output))))
    }
    fn import(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        prefix: &str,
        device: &Device,
    ) -> Result<Self> {
        self.norm1 = reader.layer_norm(self.norm1, &format!("{prefix}.norm1"), device)?;
        self.norm2 = reader.layer_norm(self.norm2, &format!("{prefix}.norm2"), device)?;
        self.qkv = reader.linear(self.qkv, &format!("{prefix}.attn.qkv"), device)?;
        self.attention_projection = reader.linear(
            self.attention_projection,
            &format!("{prefix}.attn.proj"),
            device,
        )?;
        self.mlp1 = reader.linear(self.mlp1, &format!("{prefix}.mlp.layers.0"), device)?;
        self.mlp2 = reader.linear(self.mlp2, &format!("{prefix}.mlp.layers.1"), device)?;
        if let Some(projection) = self.projection.take() {
            self.projection = Some(reader.linear(projection, &format!("{prefix}.proj"), device)?);
        }
        Ok(self)
    }
}

#[derive(Module, Debug)]
pub(crate) struct HieraImageEncoder {
    patch_embed: Conv2d,
    position: Param<Tensor<4>>,
    window_position: Param<Tensor<4>>,
    blocks: Vec<HieraBlock>,
    neck: Vec<Conv2d>,
}
impl HieraImageEncoder {
    pub(crate) fn new(device: &Device) -> Self {
        let stages = [1, 2, 7, 2];
        let windows = [8, 4, 14, 7];
        let mut blocks = Vec::new();
        let mut input = 96;
        for stage in 0..4 {
            for index in 0..stages[stage] {
                let number = blocks.len();
                let global = [5, 7, 9].contains(&number);
                let window = if global {
                    0
                } else if index == 0 && stage > 0 {
                    windows[stage - 1]
                } else {
                    windows[stage]
                };
                let output = 96 << stage;
                blocks.push(HieraBlock::new(
                    input,
                    output,
                    1 << stage,
                    window,
                    stage > 0 && index == 0,
                    device,
                ));
                input = output;
            }
        }
        Self {
            patch_embed: Conv2dConfig::new([3, 96], [7, 7])
                .with_stride([4, 4])
                .with_padding(PaddingConfig2d::Explicit(3, 3, 3, 3))
                .init(device),
            position: Initializer::Zeros.init([1, 96, 7, 7], device),
            window_position: Initializer::Zeros.init([1, 96, 8, 8], device),
            blocks,
            neck: [768, 384, 192, 96]
                .into_iter()
                .map(|channels| Conv2dConfig::new([channels, 256], [1, 1]).init(device))
                .collect(),
        }
    }
    pub(crate) fn forward(&self, input: Tensor<4>) -> Vec<Tensor<4>> {
        let patch = self.patch_embed.forward(input);
        let [_, _, height, width] = patch.dims();
        let positional = resize(
            self.position.val(),
            [height, width],
            InterpolateMode::Bicubic,
        ) + self
            .window_position
            .val()
            .repeat_dim(2, height / 8)
            .repeat_dim(3, width / 8);
        let mut output = (patch + positional).permute([0, 2, 3, 1]);
        let mut features = Vec::with_capacity(4);
        for (index, block) in self.blocks.iter().enumerate() {
            output = block.forward(output);
            if [0, 2, 9, 11].contains(&index) {
                features.push(output.clone().permute([0, 3, 1, 2]));
            }
        }
        let mut pyramid = Vec::with_capacity(4);
        let mut previous: Option<Tensor<4>> = None;
        for stage in (0..4).rev() {
            let lateral = self.neck[3 - stage].forward(features[stage].clone());
            let [_, _, height, width] = lateral.dims();
            let output = if stage >= 2 {
                match previous {
                    Some(previous) => {
                        lateral + resize(previous, [height, width], InterpolateMode::Nearest)
                    }
                    None => lateral,
                }
            } else {
                lateral
            };
            pyramid.push(output.clone());
            previous = Some(output);
        }
        pyramid.reverse();
        pyramid.pop();
        pyramid
    }
    pub(crate) fn import(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        let prefix = "image_encoder.trunk";
        self.patch_embed = reader.conv2d(
            self.patch_embed,
            &format!("{prefix}.patch_embed.proj"),
            device,
        )?;
        self.position = Param::from_tensor(reader.tensor(
            &format!("{prefix}.pos_embed"),
            [1, 96, 7, 7],
            device,
        )?);
        self.window_position = Param::from_tensor(reader.tensor(
            &format!("{prefix}.pos_embed_window"),
            [1, 96, 8, 8],
            device,
        )?);
        self.blocks = self
            .blocks
            .into_iter()
            .enumerate()
            .map(|(i, block)| block.import(reader, &format!("{prefix}.blocks.{i}"), device))
            .collect::<Result<_>>()?;
        self.neck = self
            .neck
            .into_iter()
            .enumerate()
            .map(|(i, conv)| {
                reader.conv2d(conv, &format!("image_encoder.neck.convs.{i}.conv"), device)
            })
            .collect::<Result<_>>()?;
        Ok(self)
    }
}

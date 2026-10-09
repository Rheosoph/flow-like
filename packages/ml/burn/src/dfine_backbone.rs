// Native port of D-FINE's HGNetv2-B0 and hybrid encoder.
// Copyright (c) 2024 D-FINE authors; licensed under Apache-2.0. See licenses/D-FINE.
use crate::{
    Result,
    detr_layers::ConvNorm,
    pretrained::SafeTensorReader,
    rtdetr_backbone::{EncoderLayer, position_embedding},
};
use burn::{
    module::{Module, Param},
    nn::{
        BatchNorm, BatchNormConfig, PaddingConfig2d,
        conv::{Conv2d, Conv2dConfig},
        interpolate::Interpolate2dConfig,
        pool::{MaxPool2d, MaxPool2dConfig},
    },
    tensor::{
        Device, Tensor,
        activation::{relu, silu},
    },
};

#[derive(Module, Debug)]
struct HgConv {
    conv: Conv2d,
    norm: BatchNorm,
    affine_scale: Option<Param<Tensor<1>>>,
    affine_bias: Option<Param<Tensor<1>>>,
}
impl HgConv {
    fn new(
        input: usize,
        output: usize,
        kernel: usize,
        stride: usize,
        groups: usize,
        activation: bool,
        device: &Device,
    ) -> Self {
        let padding = (kernel - 1) / 2;
        Self {
            conv: Conv2dConfig::new([input, output], [kernel, kernel])
                .with_stride([stride, stride])
                .with_groups(groups)
                .with_bias(false)
                .with_padding(PaddingConfig2d::Explicit(
                    padding, padding, padding, padding,
                ))
                .init(device),
            norm: BatchNormConfig::new(output)
                .with_epsilon(1e-5)
                .with_momentum(0.1)
                .init(device),
            affine_scale: activation.then(|| Param::from_tensor(Tensor::ones([1], device))),
            affine_bias: activation.then(|| Param::from_tensor(Tensor::zeros([1], device))),
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let result = self.norm.forward(self.conv.forward(input));
        match (&self.affine_scale, &self.affine_bias) {
            (Some(scale), Some(bias)) => {
                relu(result) * scale.val().reshape([1, 1, 1, 1]) + bias.val().reshape([1, 1, 1, 1])
            }
            _ => result,
        }
    }
    fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.conv = reader.conv2d(self.conv, &format!("{prefix}.conv"), device)?;
        self.norm = reader.batch_norm(self.norm, &format!("{prefix}.bn"), device)?;
        if self.affine_scale.is_some() {
            self.affine_scale = Some(Param::from_tensor(reader.tensor(
                &format!("{prefix}.lab.scale"),
                [1],
                device,
            )?));
            self.affine_bias = Some(Param::from_tensor(reader.tensor(
                &format!("{prefix}.lab.bias"),
                [1],
                device,
            )?));
        }
        Ok(self)
    }
}

#[derive(Module, Debug)]
struct HgLayer {
    point: Option<HgConv>,
    spatial: HgConv,
}
impl HgLayer {
    fn new(input: usize, output: usize, light: bool, device: &Device) -> Self {
        Self {
            point: light.then(|| HgConv::new(input, output, 1, 1, 1, false, device)),
            spatial: HgConv::new(
                if light { output } else { input },
                output,
                if light { 5 } else { 3 },
                1,
                if light { output } else { 1 },
                true,
                device,
            ),
        }
    }
    fn forward(&self, mut input: Tensor<4>) -> Tensor<4> {
        if let Some(point) = &self.point {
            input = point.forward(input);
        }
        self.spatial.forward(input)
    }
    fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        if let Some(point) = self.point.take() {
            self.point = Some(point.import(&format!("{prefix}.conv1"), reader, device)?);
            self.spatial = self
                .spatial
                .import(&format!("{prefix}.conv2"), reader, device)?;
        } else {
            self.spatial = self.spatial.import(prefix, reader, device)?;
        }
        Ok(self)
    }
}

#[derive(Module, Debug)]
struct HgBlock {
    layers: Vec<HgLayer>,
    squeeze: HgConv,
    expand: HgConv,
    residual: bool,
}
impl HgBlock {
    fn new(
        input: usize,
        middle: usize,
        output: usize,
        light: bool,
        residual: bool,
        device: &Device,
    ) -> Self {
        Self {
            layers: (0..3)
                .map(|i| HgLayer::new(if i == 0 { input } else { middle }, middle, light, device))
                .collect(),
            squeeze: HgConv::new(input + 3 * middle, output / 2, 1, 1, 1, true, device),
            expand: HgConv::new(output / 2, output, 1, 1, 1, true, device),
            residual,
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let mut outputs = vec![input.clone()];
        for layer in &self.layers {
            outputs.push(layer.forward(outputs.last().unwrap().clone()));
        }
        let output = self
            .expand
            .forward(self.squeeze.forward(Tensor::cat(outputs, 1)));
        if self.residual {
            output + input
        } else {
            output
        }
    }
    fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.layers = self
            .layers
            .into_iter()
            .enumerate()
            .map(|(i, layer)| layer.import(&format!("{prefix}.layers.{i}"), reader, device))
            .collect::<Result<_>>()?;
        self.squeeze = self
            .squeeze
            .import(&format!("{prefix}.aggregation.0"), reader, device)?;
        self.expand = self
            .expand
            .import(&format!("{prefix}.aggregation.1"), reader, device)?;
        Ok(self)
    }
}
#[derive(Module, Debug)]
struct HgStage {
    downsample: Option<HgConv>,
    blocks: Vec<HgBlock>,
}
impl HgStage {
    fn new(
        input: usize,
        middle: usize,
        output: usize,
        count: usize,
        downsample: bool,
        light: bool,
        device: &Device,
    ) -> Self {
        Self {
            downsample: downsample.then(|| HgConv::new(input, input, 3, 2, input, false, device)),
            blocks: (0..count)
                .map(|i| {
                    HgBlock::new(
                        if i == 0 { input } else { output },
                        middle,
                        output,
                        light,
                        i != 0,
                        device,
                    )
                })
                .collect(),
        }
    }
    fn forward(&self, mut input: Tensor<4>) -> Tensor<4> {
        if let Some(down) = &self.downsample {
            input = down.forward(input);
        }
        for block in &self.blocks {
            input = block.forward(input);
        }
        input
    }
    fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        if let Some(down) = self.downsample.take() {
            self.downsample = Some(down.import(&format!("{prefix}.downsample"), reader, device)?);
        }
        self.blocks = self
            .blocks
            .into_iter()
            .enumerate()
            .map(|(i, block)| block.import(&format!("{prefix}.blocks.{i}"), reader, device))
            .collect::<Result<_>>()?;
        Ok(self)
    }
}

#[derive(Module, Debug)]
pub(crate) struct HgNetV2B0 {
    stem1: HgConv,
    stem2a: HgConv,
    stem2b: HgConv,
    stem3: HgConv,
    stem4: HgConv,
    pool: MaxPool2d,
    stages: Vec<HgStage>,
}
impl HgNetV2B0 {
    pub(crate) fn new(device: &Device) -> Self {
        Self {
            stem1: HgConv::new(3, 16, 3, 2, 1, true, device),
            stem2a: HgConv::new(16, 8, 2, 1, 1, true, device),
            stem2b: HgConv::new(8, 16, 2, 1, 1, true, device),
            stem3: HgConv::new(32, 16, 3, 2, 1, true, device),
            stem4: HgConv::new(16, 16, 1, 1, 1, true, device),
            pool: MaxPool2dConfig::new([2, 2]).with_strides([1, 1]).init(),
            stages: vec![
                HgStage::new(16, 16, 64, 1, false, false, device),
                HgStage::new(64, 32, 256, 1, true, false, device),
                HgStage::new(256, 64, 512, 2, true, true, device),
                HgStage::new(512, 128, 1024, 1, true, true, device),
            ],
        }
    }
    pub(crate) fn forward(&self, input: Tensor<4>) -> Vec<Tensor<4>> {
        let stem = pad_bottom_right(self.stem1.forward(input));
        let branch = self
            .stem2b
            .forward(pad_bottom_right(self.stem2a.forward(stem.clone())));
        let mut input = self.stem4.forward(
            self.stem3
                .forward(Tensor::cat(vec![self.pool.forward(stem), branch], 1)),
        );
        let mut features = Vec::with_capacity(2);
        for (i, stage) in self.stages.iter().enumerate() {
            input = stage.forward(input);
            if i >= 2 {
                features.push(input.clone());
            }
        }
        features
    }
    pub(crate) fn import(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.stem1 = self.stem1.import("backbone.stem.stem1", reader, device)?;
        self.stem2a = self.stem2a.import("backbone.stem.stem2a", reader, device)?;
        self.stem2b = self.stem2b.import("backbone.stem.stem2b", reader, device)?;
        self.stem3 = self.stem3.import("backbone.stem.stem3", reader, device)?;
        self.stem4 = self.stem4.import("backbone.stem.stem4", reader, device)?;
        self.stages = self
            .stages
            .into_iter()
            .enumerate()
            .map(|(i, stage)| stage.import(&format!("backbone.stages.{i}"), reader, device))
            .collect::<Result<_>>()?;
        Ok(self)
    }
}
fn pad_bottom_right(input: Tensor<4>) -> Tensor<4> {
    let [batch, channels, height, width] = input.dims();
    let device = input.device();
    let input = Tensor::cat(
        vec![input, Tensor::zeros([batch, channels, height, 1], &device)],
        3,
    );
    Tensor::cat(
        vec![
            input,
            Tensor::zeros([batch, channels, 1, width + 1], &device),
        ],
        2,
    )
}

#[derive(Module, Debug)]
struct RepBlock {
    first: ConvNorm,
    second: ConvNorm,
}
impl RepBlock {
    fn new(channels: usize, device: &Device) -> Self {
        Self {
            first: ConvNorm::new(channels, channels, 3, 1, device),
            second: ConvNorm::new(channels, channels, 1, 1, device),
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        silu(self.first.forward(input.clone()) + self.second.forward(input))
    }
    fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.first = self
            .first
            .import(&format!("{prefix}.conv1"), reader, device)?;
        self.second = self
            .second
            .import(&format!("{prefix}.conv2"), reader, device)?;
        Ok(self)
    }
}
#[derive(Module, Debug)]
struct Csp {
    first: ConvNorm,
    second: ConvNorm,
    blocks: Vec<RepBlock>,
}
impl Csp {
    fn new(input: usize, output: usize, device: &Device) -> Self {
        Self {
            first: ConvNorm::new(input, output, 1, 1, device),
            second: ConvNorm::new(input, output, 1, 1, device),
            blocks: (0..2).map(|_| RepBlock::new(output, device)).collect(),
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let mut first = silu(self.first.forward(input.clone()));
        for block in &self.blocks {
            first = block.forward(first);
        }
        first + silu(self.second.forward(input))
    }
    fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.first = self
            .first
            .import(&format!("{prefix}.conv1"), reader, device)?;
        self.second = self
            .second
            .import(&format!("{prefix}.conv2"), reader, device)?;
        self.blocks = self
            .blocks
            .into_iter()
            .enumerate()
            .map(|(i, block)| block.import(&format!("{prefix}.bottlenecks.{i}"), reader, device))
            .collect::<Result<_>>()?;
        Ok(self)
    }
}
#[derive(Module, Debug)]
struct Elan {
    first: ConvNorm,
    second: Csp,
    second_conv: ConvNorm,
    third: Csp,
    third_conv: ConvNorm,
    output: ConvNorm,
}
impl Elan {
    fn new(device: &Device) -> Self {
        // Nano uses round(0.34 * 128 // 2) = 21 channels, and round(3 * 0.5) = 2 blocks.
        Self {
            first: ConvNorm::new(256, 256, 1, 1, device),
            second: Csp::new(128, 21, device),
            second_conv: ConvNorm::new(21, 21, 3, 1, device),
            third: Csp::new(21, 21, device),
            third_conv: ConvNorm::new(21, 21, 3, 1, device),
            output: ConvNorm::new(298, 128, 1, 1, device),
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let first = silu(self.first.forward(input));
        let [batch, _, height, width] = first.dims();
        let a = first.clone().slice([0..batch, 0..128, 0..height, 0..width]);
        let b = first.slice([0..batch, 128..256, 0..height, 0..width]);
        let c = silu(self.second_conv.forward(self.second.forward(b.clone())));
        let d = silu(self.third_conv.forward(self.third.forward(c.clone())));
        silu(self.output.forward(Tensor::cat(vec![a, b, c, d], 1)))
    }
    fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.first = self
            .first
            .import(&format!("{prefix}.cv1"), reader, device)?;
        self.second = self
            .second
            .import(&format!("{prefix}.cv2.0"), reader, device)?;
        self.second_conv = self
            .second_conv
            .import(&format!("{prefix}.cv2.1"), reader, device)?;
        self.third = self
            .third
            .import(&format!("{prefix}.cv3.0"), reader, device)?;
        self.third_conv = self
            .third_conv
            .import(&format!("{prefix}.cv3.1"), reader, device)?;
        self.output = self
            .output
            .import(&format!("{prefix}.cv4"), reader, device)?;
        Ok(self)
    }
}

#[derive(Module, Debug)]
pub(crate) struct DfineNanoEncoder {
    projections: Vec<ConvNorm>,
    encoder: EncoderLayer,
    lateral: ConvNorm,
    fpn: Elan,
    down_point: ConvNorm,
    down_spatial: ConvNorm,
    pan: Elan,
}
impl DfineNanoEncoder {
    pub(crate) fn new(device: &Device) -> Self {
        let mut down_spatial = ConvNorm::new(128, 128, 3, 2, device);
        down_spatial.conv = Conv2dConfig::new([128, 128], [3, 3])
            .with_groups(128)
            .with_stride([2, 2])
            .with_bias(false)
            .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
            .init(device);
        Self {
            projections: vec![
                ConvNorm::new(512, 128, 1, 1, device),
                ConvNorm::new(1024, 128, 1, 1, device),
            ],
            encoder: EncoderLayer::new(128, 8, device),
            lateral: ConvNorm::new(128, 128, 1, 1, device),
            fpn: Elan::new(device),
            down_point: ConvNorm::new(128, 128, 1, 1, device),
            down_spatial,
            pan: Elan::new(device),
        }
    }
    pub(crate) fn forward(&self, inputs: Vec<Tensor<4>>) -> Vec<Tensor<4>> {
        let low = self.projections[0].forward(inputs[0].clone());
        let high = self.projections[1].forward(inputs[1].clone());
        let [batch, channels, height, width] = high.dims();
        let position = position_embedding(width, height, channels, &high.device());
        let high = self
            .encoder
            .forward(
                high.reshape([batch, channels, height * width])
                    .swap_dims(1, 2),
                position,
            )
            .swap_dims(1, 2)
            .reshape([batch, channels, height, width]);
        // The upstream lateral and spatial downsample convolutions have identity activations.
        let high = self.lateral.forward(high);
        let up = Interpolate2dConfig::new()
            .with_output_size(Some([height * 2, width * 2]))
            .init()
            .forward(high.clone());
        let low = self.fpn.forward(Tensor::cat(vec![up, low], 1));
        let down = self
            .down_spatial
            .forward(self.down_point.forward(low.clone()));
        vec![low, self.pan.forward(Tensor::cat(vec![down, high], 1))]
    }
    pub(crate) fn import(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.projections = self
            .projections
            .into_iter()
            .enumerate()
            .map(|(i, projection)| {
                projection.import(&format!("encoder.input_proj.{i}"), reader, device)
            })
            .collect::<Result<_>>()?;
        self.encoder = self
            .encoder
            .import("encoder.encoder.0.layers.0", reader, device)?;
        self.lateral = self
            .lateral
            .import("encoder.lateral_convs.0", reader, device)?;
        self.fpn = self.fpn.import("encoder.fpn_blocks.0", reader, device)?;
        self.down_point =
            self.down_point
                .import("encoder.downsample_convs.0.0.cv1", reader, device)?;
        self.down_spatial =
            self.down_spatial
                .import("encoder.downsample_convs.0.0.cv2", reader, device)?;
        self.pan = self.pan.import("encoder.pan_blocks.0", reader, device)?;
        Ok(self)
    }
}

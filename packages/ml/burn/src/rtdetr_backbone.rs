// Native port of RT-DETR PResNet and HybridEncoder, Copyright (c) 2023 lyuwenyu.
// Licensed under Apache-2.0; see licenses/RT-DETR.
use crate::{
    Result,
    detr_layers::{Attention, ConvNorm, linear, norm},
    pretrained::SafeTensorReader,
};
use burn::{
    module::Module,
    nn::{
        LayerNorm, Linear, PaddingConfig2d,
        interpolate::Interpolate2dConfig,
        pool::{AvgPool2d, AvgPool2dConfig, MaxPool2d, MaxPool2dConfig},
    },
    tensor::{
        Device, Tensor, TensorData,
        activation::{gelu, relu, silu},
    },
};

#[derive(Module, Debug)]
struct ResidualBlock {
    first: ConvNorm,
    second: ConvNorm,
    third: Option<ConvNorm>,
    projection: Option<ConvNorm>,
    average: Option<AvgPool2d>,
}
impl ResidualBlock {
    fn new(
        input: usize,
        width: usize,
        stride: usize,
        shortcut: bool,
        bottleneck: bool,
        device: &Device,
    ) -> Self {
        let output = width * if bottleneck { 4 } else { 1 };
        Self {
            first: ConvNorm::new(
                input,
                width,
                if bottleneck { 1 } else { 3 },
                if bottleneck { 1 } else { stride },
                device,
            ),
            second: ConvNorm::new(width, width, 3, if bottleneck { stride } else { 1 }, device),
            third: bottleneck.then(|| ConvNorm::new(width, output, 1, 1, device)),
            projection: (!shortcut).then(|| ConvNorm::new(input, output, 1, 1, device)),
            // Inputs are multiples of 32, so the upstream ceil-mode pool has no partial windows.
            average: (!shortcut && stride == 2)
                .then(|| AvgPool2dConfig::new([2, 2]).with_strides([2, 2]).init()),
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let skip = match &self.projection {
            None => input.clone(),
            Some(projection) => projection.forward(match &self.average {
                Some(pool) => pool.forward(input.clone()),
                None => input.clone(),
            }),
        };
        let mut output = self.second.forward(relu(self.first.forward(input)));
        if let Some(third) = &self.third {
            output = third.forward(relu(output));
        }
        relu(output + skip)
    }
    fn import(
        mut self,
        prefix: &str,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.first = self
            .first
            .import(&format!("{prefix}.branch2a"), reader, device)?;
        self.second = self
            .second
            .import(&format!("{prefix}.branch2b"), reader, device)?;
        if let Some(third) = self.third.take() {
            self.third = Some(third.import(&format!("{prefix}.branch2c"), reader, device)?);
        }
        if let Some(projection) = self.projection.take() {
            let suffix = if self.average.is_some() {
                "short.conv"
            } else {
                "short"
            };
            self.projection =
                Some(projection.import(&format!("{prefix}.{suffix}"), reader, device)?);
        }
        Ok(self)
    }
}

#[derive(Module, Debug)]
pub(crate) struct PResNet {
    stem: Vec<ConvNorm>,
    pool: MaxPool2d,
    stages: Vec<Vec<ResidualBlock>>,
}
impl PResNet {
    pub(crate) fn new(bottleneck: bool, device: &Device) -> Self {
        let mut input = 64;
        let stages = (0..4)
            .map(|stage| {
                let count = if bottleneck { [3, 4, 6, 3][stage] } else { 2 };
                let width = 64 << stage;
                (0..count)
                    .map(|block| {
                        let layer = ResidualBlock::new(
                            input,
                            width,
                            if stage != 0 && block == 0 { 2 } else { 1 },
                            block != 0,
                            bottleneck,
                            device,
                        );
                        input = width * if bottleneck { 4 } else { 1 };
                        layer
                    })
                    .collect()
            })
            .collect();
        Self {
            stem: vec![
                ConvNorm::new(3, 32, 3, 2, device),
                ConvNorm::new(32, 32, 3, 1, device),
                ConvNorm::new(32, 64, 3, 1, device),
            ],
            pool: MaxPool2dConfig::new([3, 3])
                .with_strides([2, 2])
                .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
                .init(),
            stages,
        }
    }
    pub(crate) fn forward(&self, mut input: Tensor<4>) -> Vec<Tensor<4>> {
        for layer in &self.stem {
            input = relu(layer.forward(input));
        }
        input = self.pool.forward(input);
        let mut outputs = Vec::with_capacity(3);
        for (stage, blocks) in self.stages.iter().enumerate() {
            for block in blocks {
                input = block.forward(input);
            }
            if stage > 0 {
                outputs.push(input.clone());
            }
        }
        outputs
    }
    pub(crate) fn import(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.stem = self
            .stem
            .into_iter()
            .enumerate()
            .map(|(i, layer)| {
                layer.import(&format!("backbone.conv1.conv1_{}", i + 1), reader, device)
            })
            .collect::<Result<_>>()?;
        self.stages = self
            .stages
            .into_iter()
            .enumerate()
            .map(|(stage, blocks)| {
                blocks
                    .into_iter()
                    .enumerate()
                    .map(|(block, layer)| {
                        layer.import(
                            &format!("backbone.res_layers.{stage}.blocks.{block}"),
                            reader,
                            device,
                        )
                    })
                    .collect::<Result<_>>()
            })
            .collect::<Result<_>>()?;
        Ok(self)
    }
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
struct CspRep {
    first: ConvNorm,
    second: ConvNorm,
    third: Option<ConvNorm>,
    blocks: Vec<RepBlock>,
}
impl CspRep {
    fn new(channels: usize, expansion: f64, device: &Device) -> Self {
        let hidden = (channels as f64 * expansion) as usize;
        Self {
            first: ConvNorm::new(2 * channels, hidden, 1, 1, device),
            second: ConvNorm::new(2 * channels, hidden, 1, 1, device),
            third: (hidden != channels).then(|| ConvNorm::new(hidden, channels, 1, 1, device)),
            blocks: (0..3).map(|_| RepBlock::new(hidden, device)).collect(),
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let mut first = silu(self.first.forward(input.clone()));
        for block in &self.blocks {
            first = block.forward(first);
        }
        let output = first + silu(self.second.forward(input));
        self.third.as_ref().map_or_else(
            || output.clone(),
            |third| silu(third.forward(output.clone())),
        )
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
        if let Some(third) = self.third.take() {
            self.third = Some(third.import(&format!("{prefix}.conv3"), reader, device)?);
        }
        self.blocks = self
            .blocks
            .into_iter()
            .enumerate()
            .map(|(i, layer)| layer.import(&format!("{prefix}.bottlenecks.{i}"), reader, device))
            .collect::<Result<_>>()?;
        Ok(self)
    }
}

#[derive(Module, Debug)]
pub(crate) struct EncoderLayer {
    attention: Attention,
    first: Linear,
    second: Linear,
    norm1: LayerNorm,
    norm2: LayerNorm,
}
impl EncoderLayer {
    pub(crate) fn new(channels: usize, heads: usize, device: &Device) -> Self {
        Self {
            attention: Attention::new(channels, heads, device),
            first: linear(channels, channels * 4, device),
            second: linear(channels * 4, channels, device),
            norm1: norm(channels, device),
            norm2: norm(channels, device),
        }
    }
    pub(crate) fn forward(&self, input: Tensor<3>, position: Tensor<3>) -> Tensor<3> {
        let qk = input.clone() + position;
        let input = self
            .norm1
            .forward(input.clone() + self.attention.forward(qk.clone(), qk, input));
        self.norm2
            .forward(input.clone() + self.second.forward(gelu(self.first.forward(input))))
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
        self.first = reader.linear(self.first, &format!("{prefix}.linear1"), device)?;
        self.second = reader.linear(self.second, &format!("{prefix}.linear2"), device)?;
        self.norm1 = reader.layer_norm(self.norm1, &format!("{prefix}.norm1"), device)?;
        self.norm2 = reader.layer_norm(self.norm2, &format!("{prefix}.norm2"), device)?;
        Ok(self)
    }
}

#[derive(Module, Debug)]
pub(crate) struct HybridEncoder {
    projections: Vec<ConvNorm>,
    encoder: EncoderLayer,
    lateral: Vec<ConvNorm>,
    fpn: Vec<CspRep>,
    downsample: Vec<ConvNorm>,
    pan: Vec<CspRep>,
}
impl HybridEncoder {
    pub(crate) fn new(
        inputs: [usize; 3],
        channels: usize,
        heads: usize,
        expansion: f64,
        device: &Device,
    ) -> Self {
        Self {
            projections: inputs
                .into_iter()
                .map(|input| ConvNorm::new(input, channels, 1, 1, device))
                .collect(),
            encoder: EncoderLayer::new(channels, heads, device),
            lateral: (0..2)
                .map(|_| ConvNorm::new(channels, channels, 1, 1, device))
                .collect(),
            fpn: (0..2)
                .map(|_| CspRep::new(channels, expansion, device))
                .collect(),
            downsample: (0..2)
                .map(|_| ConvNorm::new(channels, channels, 3, 2, device))
                .collect(),
            pan: (0..2)
                .map(|_| CspRep::new(channels, expansion, device))
                .collect(),
        }
    }
    pub(crate) fn forward(&self, inputs: Vec<Tensor<4>>) -> Vec<Tensor<4>> {
        let mut features: Vec<_> = inputs
            .into_iter()
            .zip(&self.projections)
            .map(|(input, layer)| layer.forward(input))
            .collect();
        let [batch, channels, height, width] = features[2].dims();
        let position = position_embedding(width, height, channels, &features[2].device());
        features[2] = self
            .encoder
            .forward(
                features[2]
                    .clone()
                    .reshape([batch, channels, height * width])
                    .swap_dims(1, 2),
                position,
            )
            .swap_dims(1, 2)
            .reshape([batch, channels, height, width]);
        let mut inner = vec![features[2].clone()];
        for i in 0..2 {
            let high = silu(self.lateral[i].forward(inner[0].clone()));
            inner[0] = high.clone();
            let low = features[1 - i].clone();
            let shape = low.dims();
            let up = Interpolate2dConfig::new()
                .with_output_size(Some([shape[2], shape[3]]))
                .init()
                .forward(high);
            inner.insert(0, self.fpn[i].forward(Tensor::cat(vec![up, low], 1)));
        }
        let mut outputs = vec![inner[0].clone()];
        for i in 0..2 {
            let down = silu(self.downsample[i].forward(outputs[i].clone()));
            outputs.push(self.pan[i].forward(Tensor::cat(vec![down, inner[i + 1].clone()], 1)));
        }
        outputs
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
            .map(|(i, layer)| layer.import(&format!("encoder.input_proj.{i}"), reader, device))
            .collect::<Result<_>>()?;
        self.encoder = self
            .encoder
            .import("encoder.encoder.0.layers.0", reader, device)?;
        self.lateral = self
            .lateral
            .into_iter()
            .enumerate()
            .map(|(i, layer)| layer.import(&format!("encoder.lateral_convs.{i}"), reader, device))
            .collect::<Result<_>>()?;
        self.fpn = self
            .fpn
            .into_iter()
            .enumerate()
            .map(|(i, layer)| layer.import(&format!("encoder.fpn_blocks.{i}"), reader, device))
            .collect::<Result<_>>()?;
        self.downsample = self
            .downsample
            .into_iter()
            .enumerate()
            .map(|(i, layer)| {
                layer.import(&format!("encoder.downsample_convs.{i}"), reader, device)
            })
            .collect::<Result<_>>()?;
        self.pan = self
            .pan
            .into_iter()
            .enumerate()
            .map(|(i, layer)| layer.import(&format!("encoder.pan_blocks.{i}"), reader, device))
            .collect::<Result<_>>()?;
        Ok(self)
    }
}

pub(crate) fn position_embedding(
    width: usize,
    height: usize,
    channels: usize,
    device: &Device,
) -> Tensor<3> {
    let mut values = Vec::with_capacity(width * height * channels);
    let quarter = channels / 4;
    // Upstream RT-DETR uses width-major positions even for row-major image features.
    for x in 0..width {
        for y in 0..height {
            for (coordinate, cosine) in [(x, false), (x, true), (y, false), (y, true)] {
                for i in 0..quarter {
                    let angle = coordinate as f32 / 10_000f32.powf(i as f32 / quarter as f32);
                    values.push(if cosine { angle.cos() } else { angle.sin() });
                }
            }
        }
    }
    Tensor::from_data(
        TensorData::new(values, [1, width * height, channels]),
        device,
    )
}

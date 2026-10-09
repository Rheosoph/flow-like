use burn::{
    module::Module,
    nn::{
        BatchNorm, BatchNormConfig, Linear, LinearConfig, PaddingConfig2d,
        conv::{Conv2d, Conv2dConfig},
        interpolate::Interpolate2dConfig,
        pool::{MaxPool2d, MaxPool2dConfig},
    },
    tensor::{Device, Tensor, activation::relu},
};

#[derive(Module, Debug)]
pub struct ConvNorm {
    conv: Conv2d,
    norm: BatchNorm,
}
impl ConvNorm {
    fn import_torchvision(
        mut self,
        convolution: &str,
        normalization: &str,
        reader: &mut crate::pretrained::SafeTensorReader<'_>,
        device: &Device,
    ) -> crate::Result<Self> {
        self.conv = reader.conv2d(self.conv, convolution, device)?;
        self.norm = reader.batch_norm(self.norm, normalization, device)?;
        Ok(self)
    }
    fn new(input: usize, output: usize, kernel: usize, stride: usize, device: &Device) -> Self {
        let padding = kernel / 2;
        Self {
            conv: Conv2dConfig::new([input, output], [kernel, kernel])
                .with_stride([stride, stride])
                .with_padding(PaddingConfig2d::Explicit(
                    padding, padding, padding, padding,
                ))
                .with_bias(false)
                .init(device),
            norm: BatchNormConfig::new(output).init(device),
        }
    }
    fn forward(&self, x: Tensor<4>) -> Tensor<4> {
        self.norm.forward(self.conv.forward(x))
    }
}
#[derive(Module, Debug)]
pub struct BasicBlock {
    first: ConvNorm,
    second: ConvNorm,
    projection: Option<ConvNorm>,
}
impl BasicBlock {
    fn new(input: usize, output: usize, stride: usize, device: &Device) -> Self {
        Self {
            first: ConvNorm::new(input, output, 3, stride, device),
            second: ConvNorm::new(output, output, 3, 1, device),
            projection: (input != output || stride != 1)
                .then(|| ConvNorm::new(input, output, 1, stride, device)),
        }
    }
    fn forward(&self, x: Tensor<4>) -> Tensor<4> {
        let skip = self
            .projection
            .as_ref()
            .map_or_else(|| x.clone(), |p| p.forward(x.clone()));
        relu(self.second.forward(relu(self.first.forward(x))) + skip)
    }
}

/// Four residual stages with two basic blocks each, matching the ResNet-18 topology.
#[derive(Module, Debug)]
pub struct ResNet18 {
    stem: ConvNorm,
    pool: MaxPool2d,
    blocks: Vec<BasicBlock>,
    classifier: Linear,
}
impl ResNet18 {
    pub(crate) fn import_torchvision(
        mut self,
        reader: &mut crate::pretrained::SafeTensorReader<'_>,
        device: &Device,
    ) -> crate::Result<Self> {
        self.stem = self
            .stem
            .import_torchvision("conv1", "bn1", reader, device)?;
        self.blocks = self
            .blocks
            .into_iter()
            .enumerate()
            .map(|(index, mut block)| {
                let prefix = format!("layer{}.{}", index / 2 + 1, index % 2);
                block.first = block.first.import_torchvision(
                    &format!("{prefix}.conv1"),
                    &format!("{prefix}.bn1"),
                    reader,
                    device,
                )?;
                block.second = block.second.import_torchvision(
                    &format!("{prefix}.conv2"),
                    &format!("{prefix}.bn2"),
                    reader,
                    device,
                )?;
                if let Some(projection) = block.projection.take() {
                    block.projection = Some(projection.import_torchvision(
                        &format!("{prefix}.downsample.0"),
                        &format!("{prefix}.downsample.1"),
                        reader,
                        device,
                    )?);
                }
                Ok(block)
            })
            .collect::<crate::Result<_>>()?;
        self.classifier = reader.linear(self.classifier, "fc", device)?;
        Ok(self)
    }
    pub(crate) fn reset_classifier(mut self, classes: usize, device: &Device) -> Self {
        let input = self.classifier.weight.val().dims()[0];
        self.classifier = LinearConfig::new(input, classes).init(device);
        self
    }
    pub(crate) fn configure_fine_tuning(self, freeze_backbone: bool) -> Self {
        let mut model = if freeze_backbone {
            self.freeze()
        } else {
            self.unfreeze()
        };
        model.classifier = model.classifier.unfreeze();
        model
    }
    pub fn new(input: usize, classes: usize, width: usize, device: &Device) -> Self {
        let mut blocks = Vec::new();
        let mut channels = width;
        for stage in 0..4 {
            let output = width << stage;
            blocks.push(BasicBlock::new(
                channels,
                output,
                if stage == 0 { 1 } else { 2 },
                device,
            ));
            blocks.push(BasicBlock::new(output, output, 1, device));
            channels = output;
        }
        Self {
            stem: ConvNorm::new(input, width, 7, 2, device),
            pool: MaxPool2dConfig::new([3, 3])
                .with_strides([2, 2])
                .with_padding(PaddingConfig2d::Explicit(1, 1, 1, 1))
                .init(),
            blocks,
            classifier: LinearConfig::new(channels, classes).init(device),
        }
    }
    pub fn forward(&self, x: Tensor<4>) -> Tensor<2> {
        let features = self.features(x);
        let spatial = (features.dims()[2] * features.dims()[3]) as f32;
        self.classifier
            .forward((features.sum_dim(2).sum_dim(3) / spatial).flatten(1, 3))
    }
    pub fn features(&self, x: Tensor<4>) -> Tensor<4> {
        let mut x = self.pool.forward(relu(self.stem.forward(x)));
        for block in &self.blocks {
            x = block.forward(x);
        }
        x
    }
}

#[derive(Module, Debug)]
pub struct DoubleConv {
    first: Conv2d,
    second: Conv2d,
}
impl DoubleConv {
    fn new(input: usize, output: usize, device: &Device) -> Self {
        let conv = |a, b| {
            Conv2dConfig::new([a, b], [3, 3])
                .with_padding(PaddingConfig2d::Same)
                .init(device)
        };
        Self {
            first: conv(input, output),
            second: conv(output, output),
        }
    }
    fn forward(&self, x: Tensor<4>) -> Tensor<4> {
        relu(self.second.forward(relu(self.first.forward(x))))
    }
}
#[derive(Module, Debug)]
pub struct UNet {
    encoders: Vec<DoubleConv>,
    decoders: Vec<DoubleConv>,
    pool: MaxPool2d,
    output: Conv2d,
}
impl UNet {
    pub fn new(input: usize, classes: usize, width: usize, depth: usize, device: &Device) -> Self {
        let mut encoders = Vec::new();
        let mut previous = input;
        for level in 0..=depth {
            let channels = width << level;
            encoders.push(DoubleConv::new(previous, channels, device));
            previous = channels;
        }
        let mut decoders = Vec::new();
        for level in (0..depth).rev() {
            let channels = width << level;
            decoders.push(DoubleConv::new(previous + channels, channels, device));
            previous = channels;
        }
        Self {
            encoders,
            decoders,
            pool: MaxPool2dConfig::new([2, 2]).with_strides([2, 2]).init(),
            output: Conv2dConfig::new([width, classes], [1, 1]).init(device),
        }
    }
    pub fn forward(&self, mut x: Tensor<4>) -> Tensor<4> {
        let mut skip = Vec::new();
        for (i, block) in self.encoders.iter().enumerate() {
            x = block.forward(x);
            if i + 1 < self.encoders.len() {
                skip.push(x.clone());
                x = self.pool.forward(x);
            }
        }
        for (block, saved) in self.decoders.iter().zip(skip.into_iter().rev()) {
            let [_, _, h, w] = saved.dims();
            x = Interpolate2dConfig::new()
                .with_output_size(Some([h, w]))
                .init()
                .forward(x);
            x = block.forward(Tensor::cat(vec![x, saved], 1));
        }
        self.output.forward(x)
    }
}

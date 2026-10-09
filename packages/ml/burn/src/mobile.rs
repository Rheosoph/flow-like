use burn::{
    module::Module,
    nn::{
        BatchNorm, BatchNormConfig, Dropout, DropoutConfig, Linear, LinearConfig, PaddingConfig2d,
        conv::{Conv2d, Conv2dConfig},
    },
    tensor::{Device, Tensor, activation::sigmoid},
};

fn channels(value: usize, width: f64) -> usize {
    (((value as f64 * width) as usize + 7) / 8 * 8).max(8)
}
#[derive(Module, Debug)]
struct ConvBn {
    conv: Conv2d,
    norm: BatchNorm,
}
impl ConvBn {
    fn new(
        input: usize,
        output: usize,
        kernel: usize,
        stride: usize,
        groups: usize,
        device: &Device,
    ) -> Self {
        Self {
            conv: Conv2dConfig::new([input, output], [kernel, kernel])
                .with_stride([stride, stride])
                .with_groups(groups)
                .with_padding(PaddingConfig2d::Same)
                .with_bias(false)
                .init(device),
            norm: BatchNormConfig::new(output).init(device),
        }
    }
    fn forward(&self, x: Tensor<4>) -> Tensor<4> {
        self.norm.forward(self.conv.forward(x))
    }
}
fn relu6(x: Tensor<4>) -> Tensor<4> {
    x.clamp(0.0, 6.0)
}
fn swish(x: Tensor<4>) -> Tensor<4> {
    x.clone() * sigmoid(x)
}
#[derive(Module, Debug)]
struct InvertedResidual {
    expand: Option<ConvBn>,
    depthwise: ConvBn,
    project: ConvBn,
    residual: bool,
}
impl InvertedResidual {
    fn new(input: usize, output: usize, stride: usize, expansion: usize, device: &Device) -> Self {
        let hidden = input * expansion;
        Self {
            expand: (expansion != 1).then(|| ConvBn::new(input, hidden, 1, 1, 1, device)),
            depthwise: ConvBn::new(hidden, hidden, 3, stride, hidden, device),
            project: ConvBn::new(hidden, output, 1, 1, 1, device),
            residual: input == output && stride == 1,
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let x = self
            .expand
            .as_ref()
            .map_or_else(|| input.clone(), |m| relu6(m.forward(input.clone())));
        let x = self.project.forward(relu6(self.depthwise.forward(x)));
        if self.residual { x + input } else { x }
    }
}
#[derive(Module, Debug)]
pub struct MobileNetV2 {
    stem: ConvBn,
    blocks: Vec<InvertedResidual>,
    last: ConvBn,
    dropout: Dropout,
    head: Linear,
}
impl MobileNetV2 {
    pub(crate) fn reset_classifier(mut self, classes: usize, device: &Device) -> Self {
        let input = self.head.weight.val().dims()[0];
        self.head = LinearConfig::new(input, classes).init(device);
        self
    }
    pub(crate) fn configure_fine_tuning(self, freeze_backbone: bool) -> Self {
        let mut model = if freeze_backbone {
            self.freeze()
        } else {
            self.unfreeze()
        };
        model.head = model.head.unfreeze();
        model.dropout = model.dropout.unfreeze();
        model
    }
    pub fn new(input: usize, classes: usize, width: f64, device: &Device) -> Self {
        let first = channels(32, width);
        let mut previous = first;
        let mut blocks = Vec::new();
        for (expand, c, n, s) in [
            (1, 16, 1, 1),
            (6, 24, 2, 2),
            (6, 32, 3, 2),
            (6, 64, 4, 2),
            (6, 96, 3, 1),
            (6, 160, 3, 2),
            (6, 320, 1, 1),
        ] {
            let output = channels(c, width);
            for i in 0..n {
                blocks.push(InvertedResidual::new(
                    previous,
                    output,
                    if i == 0 { s } else { 1 },
                    expand,
                    device,
                ));
                previous = output;
            }
        }
        let last = channels(1280, width.max(1.0));
        Self {
            stem: ConvBn::new(input, first, 3, 2, 1, device),
            blocks,
            last: ConvBn::new(previous, last, 1, 1, 1, device),
            dropout: DropoutConfig::new(0.2).init(),
            head: LinearConfig::new(last, classes).init(device),
        }
    }
    pub fn forward(&self, input: Tensor<4>) -> Tensor<2> {
        let x = self.features(input).mean_dim(2).mean_dim(3).flatten(1, 3);
        self.head.forward(self.dropout.forward(x))
    }
    pub fn features(&self, input: Tensor<4>) -> Tensor<4> {
        let mut x = relu6(self.stem.forward(input));
        for block in &self.blocks {
            x = block.forward(x);
        }
        relu6(self.last.forward(x))
    }
}

#[derive(Module, Debug)]
struct SqueezeExcitation {
    reduce: Conv2d,
    expand: Conv2d,
}
impl SqueezeExcitation {
    fn new(channels: usize, squeeze: usize, device: &Device) -> Self {
        Self {
            reduce: Conv2dConfig::new([channels, squeeze], [1, 1]).init(device),
            expand: Conv2dConfig::new([squeeze, channels], [1, 1]).init(device),
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let scale = input.clone().mean_dim(2).mean_dim(3);
        input * sigmoid(self.expand.forward(swish(self.reduce.forward(scale))))
    }
}
#[derive(Module, Debug)]
struct MBConv {
    expand: Option<ConvBn>,
    depthwise: ConvBn,
    se: SqueezeExcitation,
    project: ConvBn,
    residual: bool,
}
impl MBConv {
    fn new(
        input: usize,
        output: usize,
        kernel: usize,
        stride: usize,
        expansion: usize,
        device: &Device,
    ) -> Self {
        let hidden = input * expansion;
        Self {
            expand: (expansion != 1).then(|| ConvBn::new(input, hidden, 1, 1, 1, device)),
            depthwise: ConvBn::new(hidden, hidden, kernel, stride, hidden, device),
            se: SqueezeExcitation::new(hidden, (input / 4).max(1), device),
            project: ConvBn::new(hidden, output, 1, 1, 1, device),
            residual: input == output && stride == 1,
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let x = self
            .expand
            .as_ref()
            .map_or_else(|| input.clone(), |m| swish(m.forward(input.clone())));
        let x = self
            .project
            .forward(self.se.forward(swish(self.depthwise.forward(x))));
        if self.residual { x + input } else { x }
    }
}
/// EfficientNet-B0 stage topology with configurable width; stochastic depth is not enabled.
#[derive(Module, Debug)]
pub struct EfficientNet {
    stem: ConvBn,
    blocks: Vec<MBConv>,
    last: ConvBn,
    dropout: Dropout,
    head: Linear,
}
impl EfficientNet {
    pub(crate) fn reset_classifier(mut self, classes: usize, device: &Device) -> Self {
        let input = self.head.weight.val().dims()[0];
        self.head = LinearConfig::new(input, classes).init(device);
        self
    }
    pub(crate) fn configure_fine_tuning(self, freeze_backbone: bool) -> Self {
        let mut model = if freeze_backbone {
            self.freeze()
        } else {
            self.unfreeze()
        };
        model.head = model.head.unfreeze();
        model.dropout = model.dropout.unfreeze();
        model
    }
    pub fn new(input: usize, classes: usize, width: f64, device: &Device) -> Self {
        let first = channels(32, width);
        let mut previous = first;
        let mut blocks = Vec::new();
        for (expand, k, c, n, s) in [
            (1, 3, 16, 1, 1),
            (6, 3, 24, 2, 2),
            (6, 5, 40, 2, 2),
            (6, 3, 80, 3, 2),
            (6, 5, 112, 3, 1),
            (6, 5, 192, 4, 2),
            (6, 3, 320, 1, 1),
        ] {
            let output = channels(c, width);
            for i in 0..n {
                blocks.push(MBConv::new(
                    previous,
                    output,
                    k,
                    if i == 0 { s } else { 1 },
                    expand,
                    device,
                ));
                previous = output;
            }
        }
        let last = channels(1280, width);
        Self {
            stem: ConvBn::new(input, first, 3, 2, 1, device),
            blocks,
            last: ConvBn::new(previous, last, 1, 1, 1, device),
            dropout: DropoutConfig::new(0.2).init(),
            head: LinearConfig::new(last, classes).init(device),
        }
    }
    pub fn forward(&self, input: Tensor<4>) -> Tensor<2> {
        let x = self.features(input).mean_dim(2).mean_dim(3).flatten(1, 3);
        self.head.forward(self.dropout.forward(x))
    }
    pub fn features(&self, input: Tensor<4>) -> Tensor<4> {
        let mut x = swish(self.stem.forward(input));
        for block in &self.blocks {
            x = block.forward(x);
        }
        swish(self.last.forward(x))
    }
}

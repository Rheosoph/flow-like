use burn::{
    module::Module,
    nn::{
        Gru, GruConfig, Linear, LinearConfig, Lstm, LstmConfig, PaddingConfig1d,
        conv::{Conv1d, Conv1dConfig},
    },
    tensor::{Device, Tensor, activation::relu},
};

#[derive(Module, Debug)]
pub struct Mlp {
    input: Linear,
    output: Linear,
}
impl Mlp {
    pub fn new(input: usize, hidden: usize, output: usize, device: &Device) -> Self {
        Self {
            input: LinearConfig::new(input, hidden).init(device),
            output: LinearConfig::new(hidden, output).init(device),
        }
    }
    pub fn forward(&self, x: Tensor<2>) -> Tensor<2> {
        self.output.forward(relu(self.input.forward(x)))
    }
}

#[derive(Module, Debug)]
pub enum RecurrentLayer {
    Lstm(Lstm),
    Gru(Gru),
}
#[derive(Module, Debug)]
pub struct Recurrent {
    layer: RecurrentLayer,
    output: Linear,
}
impl Recurrent {
    pub fn new(input: usize, hidden: usize, output: usize, lstm: bool, device: &Device) -> Self {
        let layer = if lstm {
            RecurrentLayer::Lstm(LstmConfig::new(input, hidden, true).init(device))
        } else {
            RecurrentLayer::Gru(GruConfig::new(input, hidden, true).init(device))
        };
        Self {
            layer,
            output: LinearConfig::new(hidden, output).init(device),
        }
    }
    pub fn forward(&self, x: Tensor<3>) -> Tensor<2> {
        let sequence = match &self.layer {
            RecurrentLayer::Lstm(m) => m.forward(x, None).0,
            RecurrentLayer::Gru(m) => m.forward(x, None),
        };
        let last = sequence.dims()[1] - 1;
        self.output
            .forward(sequence.narrow(1, last, 1).squeeze_dim(1))
    }
}

#[derive(Module, Debug)]
pub struct Cnn1d {
    first: Conv1d,
    second: Conv1d,
    output: Linear,
}
impl Cnn1d {
    pub fn new(input: usize, hidden: usize, output: usize, device: &Device) -> Self {
        Self {
            first: Conv1dConfig::new(input, hidden, 3)
                .with_padding(PaddingConfig1d::Same)
                .init(device),
            second: Conv1dConfig::new(hidden, hidden, 3)
                .with_padding(PaddingConfig1d::Same)
                .init(device),
            output: LinearConfig::new(hidden, output).init(device),
        }
    }
    pub fn forward(&self, x: Tensor<3>) -> Tensor<2> {
        let x = relu(self.first.forward(x.swap_dims(1, 2)));
        let x = relu(self.second.forward(x)).mean_dim(2).squeeze_dim(2);
        self.output.forward(x)
    }
}

#[derive(Module, Debug)]
pub struct TemporalBlock {
    first: Conv1d,
    second: Conv1d,
    projection: Option<Conv1d>,
}
impl TemporalBlock {
    fn new(input: usize, hidden: usize, dilation: usize, device: &Device) -> Self {
        let conv = |a, b| {
            Conv1dConfig::new(a, b, 3)
                .with_dilation(dilation)
                .with_padding(PaddingConfig1d::Explicit(2 * dilation, 0))
                .init(device)
        };
        Self {
            first: conv(input, hidden),
            second: conv(hidden, hidden),
            projection: (input != hidden).then(|| Conv1dConfig::new(input, hidden, 1).init(device)),
        }
    }
    fn forward(&self, x: Tensor<3>) -> Tensor<3> {
        let skip = self
            .projection
            .as_ref()
            .map_or_else(|| x.clone(), |p| p.forward(x.clone()));
        relu(self.second.forward(relu(self.first.forward(x))) + skip)
    }
}
#[derive(Module, Debug)]
pub struct Tcn {
    blocks: Vec<TemporalBlock>,
    output: Linear,
}
impl Tcn {
    pub fn new(
        input: usize,
        hidden: usize,
        levels: usize,
        outputs: usize,
        device: &Device,
    ) -> Self {
        let blocks = (0..levels)
            .map(|i| {
                TemporalBlock::new(
                    if i == 0 { input } else { hidden },
                    hidden,
                    1usize << i,
                    device,
                )
            })
            .collect();
        Self {
            blocks,
            output: LinearConfig::new(hidden, outputs).init(device),
        }
    }
    pub fn forward(&self, x: Tensor<3>) -> Tensor<2> {
        let mut x = x.swap_dims(1, 2);
        for block in &self.blocks {
            x = block.forward(x);
        }
        let last = x.dims()[2] - 1;
        self.output.forward(x.narrow(2, last, 1).squeeze_dim(2))
    }
}

#[derive(Module, Debug)]
pub struct DenseAutoencoder {
    encoder: Linear,
    latent: Linear,
    decoder: Linear,
    output: Linear,
}
impl DenseAutoencoder {
    pub fn new(input: usize, hidden: usize, latent: usize, device: &Device) -> Self {
        Self {
            encoder: LinearConfig::new(input, hidden).init(device),
            latent: LinearConfig::new(hidden, latent).init(device),
            decoder: LinearConfig::new(latent, hidden).init(device),
            output: LinearConfig::new(hidden, input).init(device),
        }
    }
    pub fn forward(&self, x: Tensor<2>) -> Tensor<2> {
        let x = relu(self.encoder.forward(x));
        let x = self.latent.forward(x);
        self.output.forward(relu(self.decoder.forward(x)))
    }
}
#[derive(Module, Debug)]
pub struct ConvAutoencoder {
    encoder: Conv1d,
    latent: Conv1d,
    decoder: Conv1d,
    output: Conv1d,
}
impl ConvAutoencoder {
    pub fn new(input: usize, hidden: usize, latent: usize, device: &Device) -> Self {
        let conv = |a, b| {
            Conv1dConfig::new(a, b, 3)
                .with_padding(PaddingConfig1d::Same)
                .init(device)
        };
        Self {
            encoder: conv(input, hidden),
            latent: conv(hidden, latent),
            decoder: conv(latent, hidden),
            output: conv(hidden, input),
        }
    }
    pub fn forward(&self, x: Tensor<3>) -> Tensor<3> {
        let x = relu(self.encoder.forward(x.swap_dims(1, 2)));
        let x = self.latent.forward(x);
        self.output
            .forward(relu(self.decoder.forward(x)))
            .swap_dims(1, 2)
    }
}

#[derive(Module, Debug)]
pub struct LstmAutoencoder {
    encoder: Lstm,
    bottleneck: Linear,
    decoder: Lstm,
    output: Linear,
}
impl LstmAutoencoder {
    pub fn new(input: usize, hidden: usize, latent: usize, device: &Device) -> Self {
        Self {
            encoder: LstmConfig::new(input, hidden, true).init(device),
            bottleneck: LinearConfig::new(hidden, latent).init(device),
            decoder: LstmConfig::new(latent, hidden, true).init(device),
            output: LinearConfig::new(hidden, input).init(device),
        }
    }
    pub fn forward(&self, input: Tensor<3>) -> Tensor<3> {
        let steps = input.dims()[1];
        let state = self.encoder.forward(input, None).1;
        let latent = self.bottleneck.forward(state.hidden);
        let sequence = latent.unsqueeze_dim::<3>(1).repeat_dim(1, steps);
        self.output.forward(self.decoder.forward(sequence, None).0)
    }
}

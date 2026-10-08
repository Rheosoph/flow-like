use burn::{
    module::Module,
    nn::{
        Linear, LinearConfig, Lstm, LstmConfig, PaddingConfig2d,
        conv::{Conv2d, Conv2dConfig},
        pool::{MaxPool2d, MaxPool2dConfig},
    },
    tensor::{Device, Tensor, activation::relu},
};
#[derive(Module, Debug)]
struct ImageEncoder {
    first: Conv2d,
    second: Conv2d,
    third: Conv2d,
    pool: MaxPool2d,
}
impl ImageEncoder {
    fn new(input: usize, width: usize, device: &Device) -> Self {
        let conv = |a, b| {
            Conv2dConfig::new([a, b], [3, 3])
                .with_padding(PaddingConfig2d::Same)
                .init(device)
        };
        Self {
            first: conv(input, width),
            second: conv(width, width * 2),
            third: conv(width * 2, width * 4),
            pool: MaxPool2dConfig::new([2, 2]).with_strides([2, 2]).init(),
        }
    }
    fn forward(&self, x: Tensor<4>) -> Tensor<2> {
        let x = self.pool.forward(relu(self.first.forward(x)));
        let x = self.pool.forward(relu(self.second.forward(x)));
        relu(self.third.forward(x))
            .mean_dim(2)
            .mean_dim(3)
            .flatten(1, 3)
    }
}
#[derive(Module, Debug)]
pub struct CnnLstm {
    encoder: ImageEncoder,
    lstm: Lstm,
    output: Linear,
}
impl CnnLstm {
    pub fn new(input: usize, cnn: usize, hidden: usize, outputs: usize, device: &Device) -> Self {
        Self {
            encoder: ImageEncoder::new(input, cnn, device),
            lstm: LstmConfig::new(cnn * 4, hidden, true).init(device),
            output: LinearConfig::new(hidden, outputs).init(device),
        }
    }
    pub fn forward(&self, x: Tensor<5>) -> Tensor<2> {
        let [batch, time, c, h, w] = x.dims();
        let features = self.encoder.forward(x.reshape([batch * time, c, h, w]));
        let hidden = features.dims()[1];
        let encoded = features.reshape([batch, time, hidden]);
        self.output
            .forward(self.lstm.forward(encoded, None).1.hidden)
    }
}
/// Images are flattened in NCHW order, followed by sensor features for each sample.
#[derive(Module, Debug)]
pub struct ImageSensorFusion {
    encoder: ImageEncoder,
    sensor: Linear,
    fusion: Linear,
    output: Linear,
    channels: usize,
    height: usize,
    width: usize,
    sensor_features: usize,
}
impl ImageSensorFusion {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        channels: usize,
        height: usize,
        width: usize,
        sensor_features: usize,
        cnn: usize,
        hidden: usize,
        outputs: usize,
        device: &Device,
    ) -> Self {
        Self {
            encoder: ImageEncoder::new(channels, cnn, device),
            sensor: LinearConfig::new(sensor_features, hidden).init(device),
            fusion: LinearConfig::new(cnn * 4 + hidden, hidden).init(device),
            output: LinearConfig::new(hidden, outputs).init(device),
            channels,
            height,
            width,
            sensor_features,
        }
    }
    pub fn forward(&self, x: Tensor<2>) -> Tensor<2> {
        let batch = x.dims()[0];
        let image_features = self.channels * self.height * self.width;
        let images = x.clone().slice([0..batch, 0..image_features]).reshape([
            batch,
            self.channels,
            self.height,
            self.width,
        ]);
        let sensors = x.slice([
            0..batch,
            image_features..image_features + self.sensor_features,
        ]);
        let features = Tensor::cat(
            vec![
                self.encoder.forward(images),
                relu(self.sensor.forward(sensors)),
            ],
            1,
        );
        self.output.forward(relu(self.fusion.forward(features)))
    }
}

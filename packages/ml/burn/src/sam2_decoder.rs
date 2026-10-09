// Copyright (c) Meta Platforms, Inc. and affiliates. All rights reserved.
// Adapted from SAM 2.1 under Apache-2.0; see licenses/SAM2.
use crate::{Result, Sam2Prompt, pretrained::SafeTensorReader};
use burn::{
    module::{Module, Param},
    nn::{
        Initializer, LayerNorm, LayerNormConfig, Linear, LinearConfig,
        conv::{Conv2d, Conv2dConfig, ConvTranspose2d, ConvTranspose2dConfig},
    },
    tensor::{
        Device, Tensor, TensorData,
        activation::{gelu, relu, sigmoid, softmax},
    },
};

#[derive(Module, Debug)]
struct ChannelNorm {
    weight: Param<Tensor<1>>,
    bias: Param<Tensor<1>>,
}
impl ChannelNorm {
    fn new(channels: usize, device: &Device) -> Self {
        Self {
            weight: Initializer::Ones.init([channels], device),
            bias: Initializer::Zeros.init([channels], device),
        }
    }
    fn forward(&self, input: Tensor<4>) -> Tensor<4> {
        let centered = input.clone() - input.mean_dim(1);
        let variance = centered.clone().powf_scalar(2.0).mean_dim(1);
        let channels = self.weight.dims()[0];
        centered / (variance + 1e-6).sqrt() * self.weight.val().reshape([1, channels, 1, 1])
            + self.bias.val().reshape([1, channels, 1, 1])
    }
    fn import(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        prefix: &str,
        device: &Device,
    ) -> Result<Self> {
        let shape = self.weight.dims();
        self.weight =
            Param::from_tensor(reader.tensor(&format!("{prefix}.weight"), shape, device)?);
        self.bias = Param::from_tensor(reader.tensor(&format!("{prefix}.bias"), shape, device)?);
        Ok(self)
    }
}

#[derive(Module, Debug)]
pub(crate) struct PromptEncoder {
    gaussian: Param<Tensor<2>>,
    points: Vec<Param<Tensor<2>>>,
    not_a_point: Param<Tensor<2>>,
    no_mask: Param<Tensor<2>>,
    mask_conv1: Conv2d,
    mask_norm1: ChannelNorm,
    mask_conv2: Conv2d,
    mask_norm2: ChannelNorm,
    mask_conv3: Conv2d,
}
impl PromptEncoder {
    pub(crate) fn new(device: &Device) -> Self {
        let embedding = || {
            Initializer::Normal {
                mean: 0.0,
                std: 1.0,
            }
            .init([1, 256], device)
        };
        Self {
            gaussian: Initializer::Normal {
                mean: 0.0,
                std: 1.0,
            }
            .init([2, 128], device),
            points: (0..4).map(|_| embedding()).collect(),
            not_a_point: embedding(),
            no_mask: embedding(),
            mask_conv1: Conv2dConfig::new([1, 4], [2, 2])
                .with_stride([2, 2])
                .init(device),
            mask_norm1: ChannelNorm::new(4, device),
            mask_conv2: Conv2dConfig::new([4, 16], [2, 2])
                .with_stride([2, 2])
                .init(device),
            mask_norm2: ChannelNorm::new(16, device),
            mask_conv3: Conv2dConfig::new([16, 256], [1, 1]).init(device),
        }
    }
    fn position(&self, coordinates: Tensor<2>) -> Tensor<2> {
        let phase = ((coordinates * 2.0 - 1.0).matmul(self.gaussian.val().detach()))
            * (2.0 * std::f64::consts::PI);
        Tensor::cat(vec![phase.clone().sin(), phase.cos()], 1)
    }
    pub(crate) fn dense_position(&self, grid: usize, device: &Device) -> Tensor<4> {
        let mut coords = Vec::with_capacity(grid * grid * 2);
        for y in 0..grid {
            for x in 0..grid {
                coords.extend([
                    (x as f32 + 0.5) / grid as f32,
                    (y as f32 + 0.5) / grid as f32,
                ]);
            }
        }
        self.position(Tensor::from_data(
            TensorData::new(coords, [grid * grid, 2]),
            device,
        ))
        .transpose()
        .reshape([1, 256, grid, grid])
    }
    pub(crate) fn forward(
        &self,
        prompt: &Sam2Prompt,
        grid: usize,
        device: &Device,
    ) -> (Option<Tensor<3>>, Tensor<4>) {
        let mut embeddings = Vec::new();
        // SAM2ImagePredictor merges box corners into point tokens, then adds one padding point.
        if let Some([x0, y0, x1, y1]) = prompt.bbox {
            for (index, [x, y]) in [[x0, y0], [x1, y1]].into_iter().enumerate() {
                let coord = Tensor::from_data([[(x + 0.5) / 1024.0, (y + 0.5) / 1024.0]], device);
                embeddings.push(self.position(coord) + self.points[index + 2].val());
            }
        }
        for point in &prompt.points {
            let coord = Tensor::from_data(
                [[(point.x + 0.5) / 1024.0, (point.y + 0.5) / 1024.0]],
                device,
            );
            embeddings.push(self.position(coord) + self.points[point.label as usize].val());
        }
        if !embeddings.is_empty() {
            embeddings.push(self.not_a_point.val());
        }
        let sparse = (!embeddings.is_empty()).then(|| Tensor::cat(embeddings, 0).unsqueeze_dim(0));
        let dense = if let Some(mask) = &prompt.mask {
            let input = Tensor::from_data(
                TensorData::new(mask.values.clone(), [1, 1, grid * 4, grid * 4]),
                device,
            );
            self.mask_conv3
                .forward(gelu(self.mask_norm2.forward(self.mask_conv2.forward(
                    gelu(self.mask_norm1.forward(self.mask_conv1.forward(input))),
                ))))
        } else {
            self.no_mask
                .val()
                .reshape([1, 256, 1, 1])
                .expand([1, 256, grid, grid])
        };
        (sparse, dense)
    }
    pub(crate) fn import(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        let p = "sam_prompt_encoder";
        self.gaussian = Param::from_tensor(reader.tensor(
            &format!("{p}.pe_layer.positional_encoding_gaussian_matrix"),
            [2, 128],
            device,
        )?);
        for (index, point) in self.points.iter_mut().enumerate() {
            *point = Param::from_tensor(reader.tensor(
                &format!("{p}.point_embeddings.{index}.weight"),
                [1, 256],
                device,
            )?);
        }
        self.not_a_point = Param::from_tensor(reader.tensor(
            &format!("{p}.not_a_point_embed.weight"),
            [1, 256],
            device,
        )?);
        self.no_mask = Param::from_tensor(reader.tensor(
            &format!("{p}.no_mask_embed.weight"),
            [1, 256],
            device,
        )?);
        self.mask_conv1 =
            reader.conv2d(self.mask_conv1, &format!("{p}.mask_downscaling.0"), device)?;
        self.mask_norm1 =
            self.mask_norm1
                .import(reader, &format!("{p}.mask_downscaling.1"), device)?;
        self.mask_conv2 =
            reader.conv2d(self.mask_conv2, &format!("{p}.mask_downscaling.3"), device)?;
        self.mask_norm2 =
            self.mask_norm2
                .import(reader, &format!("{p}.mask_downscaling.4"), device)?;
        self.mask_conv3 =
            reader.conv2d(self.mask_conv3, &format!("{p}.mask_downscaling.6"), device)?;
        Ok(self)
    }
}

#[derive(Module, Debug)]
struct Attention {
    query: Linear,
    key: Linear,
    value: Linear,
    output: Linear,
}
impl Attention {
    fn new(downsample: usize, device: &Device) -> Self {
        let dim = 256 / downsample;
        Self {
            query: LinearConfig::new(256, dim).init(device),
            key: LinearConfig::new(256, dim).init(device),
            value: LinearConfig::new(256, dim).init(device),
            output: LinearConfig::new(dim, 256).init(device),
        }
    }
    fn forward(&self, query: Tensor<3>, key: Tensor<3>, value: Tensor<3>) -> Tensor<3> {
        let [batch, q_tokens, _] = query.dims();
        let tokens = key.dims()[1];
        let dim = self.query.weight.dims()[1];
        let head = dim / 8;
        let q = self
            .query
            .forward(query)
            .reshape([batch, q_tokens, 8, head])
            .permute([0, 2, 1, 3]);
        let k = self
            .key
            .forward(key)
            .reshape([batch, tokens, 8, head])
            .permute([0, 2, 3, 1]);
        let v = self
            .value
            .forward(value)
            .reshape([batch, tokens, 8, head])
            .permute([0, 2, 1, 3]);
        self.output.forward(
            softmax(q.matmul(k) / (head as f64).sqrt(), 3)
                .matmul(v)
                .permute([0, 2, 1, 3])
                .reshape([batch, q_tokens, dim]),
        )
    }
    fn import(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        prefix: &str,
        device: &Device,
    ) -> Result<Self> {
        self.query = reader.linear(self.query, &format!("{prefix}.q_proj"), device)?;
        self.key = reader.linear(self.key, &format!("{prefix}.k_proj"), device)?;
        self.value = reader.linear(self.value, &format!("{prefix}.v_proj"), device)?;
        self.output = reader.linear(self.output, &format!("{prefix}.out_proj"), device)?;
        Ok(self)
    }
}
#[derive(Module, Debug)]
struct Mlp {
    layers: Vec<Linear>,
}
impl Mlp {
    fn new(input: usize, hidden: usize, output: usize, depth: usize, device: &Device) -> Self {
        Self {
            layers: (0..depth)
                .map(|index| {
                    LinearConfig::new(
                        if index == 0 { input } else { hidden },
                        if index + 1 == depth { output } else { hidden },
                    )
                    .init(device)
                })
                .collect(),
        }
    }
    fn forward<const D: usize>(&self, mut input: Tensor<D>) -> Tensor<D> {
        for (index, layer) in self.layers.iter().enumerate() {
            input = layer.forward(input);
            if index + 1 < self.layers.len() {
                input = relu(input);
            }
        }
        input
    }
    fn import(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        prefix: &str,
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
struct TwoWayBlock {
    self_attention: Attention,
    token_to_image: Attention,
    image_to_token: Attention,
    norms: Vec<LayerNorm>,
    mlp: Mlp,
    first: bool,
}
impl TwoWayBlock {
    fn new(first: bool, device: &Device) -> Self {
        Self {
            self_attention: Attention::new(1, device),
            token_to_image: Attention::new(2, device),
            image_to_token: Attention::new(2, device),
            norms: (0..4)
                .map(|_| LayerNormConfig::new(256).with_epsilon(1e-5).init(device))
                .collect(),
            mlp: Mlp::new(256, 2048, 256, 2, device),
            first,
        }
    }
    fn forward(
        &self,
        mut queries: Tensor<3>,
        mut keys: Tensor<3>,
        q_position: Tensor<3>,
        k_position: Tensor<3>,
    ) -> (Tensor<3>, Tensor<3>) {
        queries = if self.first {
            self.self_attention
                .forward(queries.clone(), queries.clone(), queries)
        } else {
            let q = queries.clone() + q_position.clone();
            queries.clone() + self.self_attention.forward(q.clone(), q, queries)
        };
        queries = self.norms[0].forward(queries);
        let attended = self.token_to_image.forward(
            queries.clone() + q_position.clone(),
            keys.clone() + k_position.clone(),
            keys.clone(),
        );
        queries = self.norms[1].forward(queries + attended);
        queries = self.norms[2].forward(queries.clone() + self.mlp.forward(queries));
        let attended = self.image_to_token.forward(
            keys.clone() + k_position,
            queries.clone() + q_position,
            queries.clone(),
        );
        keys = self.norms[3].forward(keys + attended);
        (queries, keys)
    }
    fn import(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        prefix: &str,
        device: &Device,
    ) -> Result<Self> {
        self.self_attention =
            self.self_attention
                .import(reader, &format!("{prefix}.self_attn"), device)?;
        self.token_to_image = self.token_to_image.import(
            reader,
            &format!("{prefix}.cross_attn_token_to_image"),
            device,
        )?;
        self.image_to_token = self.image_to_token.import(
            reader,
            &format!("{prefix}.cross_attn_image_to_token"),
            device,
        )?;
        self.norms = self
            .norms
            .into_iter()
            .enumerate()
            .map(|(i, norm)| reader.layer_norm(norm, &format!("{prefix}.norm{}", i + 1), device))
            .collect::<Result<_>>()?;
        self.mlp = self.mlp.import(reader, &format!("{prefix}.mlp"), device)?;
        Ok(self)
    }
}

#[derive(Clone, Debug)]
pub struct Sam2Output {
    /// Token zero gives the single mask; tokens one through three give alternative masks.
    pub mask_logits: Tensor<4>,
    pub iou: Tensor<2>,
    pub object_score_logits: Tensor<2>,
}
#[derive(Module, Debug)]
pub(crate) struct MaskDecoder {
    object_token: Param<Tensor<2>>,
    iou_token: Param<Tensor<2>>,
    mask_tokens: Param<Tensor<2>>,
    blocks: Vec<TwoWayBlock>,
    final_attention: Attention,
    final_norm: LayerNorm,
    up1: ConvTranspose2d,
    up_norm: ChannelNorm,
    up2: ConvTranspose2d,
    high0: Conv2d,
    high1: Conv2d,
    hypernetworks: Vec<Mlp>,
    iou_head: Mlp,
    object_head: Mlp,
}
impl MaskDecoder {
    pub(crate) fn new(device: &Device) -> Self {
        Self {
            object_token: Initializer::Normal {
                mean: 0.0,
                std: 1.0,
            }
            .init([1, 256], device),
            iou_token: Initializer::Normal {
                mean: 0.0,
                std: 1.0,
            }
            .init([1, 256], device),
            mask_tokens: Initializer::Normal {
                mean: 0.0,
                std: 1.0,
            }
            .init([4, 256], device),
            blocks: vec![
                TwoWayBlock::new(true, device),
                TwoWayBlock::new(false, device),
            ],
            final_attention: Attention::new(2, device),
            final_norm: LayerNormConfig::new(256).with_epsilon(1e-5).init(device),
            up1: ConvTranspose2dConfig::new([256, 64], [2, 2])
                .with_stride([2, 2])
                .init(device),
            up_norm: ChannelNorm::new(64, device),
            up2: ConvTranspose2dConfig::new([64, 32], [2, 2])
                .with_stride([2, 2])
                .init(device),
            high0: Conv2dConfig::new([256, 32], [1, 1]).init(device),
            high1: Conv2dConfig::new([256, 64], [1, 1]).init(device),
            hypernetworks: (0..4).map(|_| Mlp::new(256, 256, 32, 3, device)).collect(),
            iou_head: Mlp::new(256, 256, 4, 3, device),
            object_head: Mlp::new(256, 256, 1, 3, device),
        }
    }
    pub(crate) fn forward(
        &self,
        features: &[Tensor<4>],
        position: Tensor<4>,
        sparse: Option<Tensor<3>>,
        dense: Tensor<4>,
    ) -> Sam2Output {
        let [batch, _, height, width] = features[2].dims();
        let tokens = Tensor::cat(
            vec![
                self.object_token.val(),
                self.iou_token.val(),
                self.mask_tokens.val(),
            ],
            0,
        )
        .unsqueeze_dim::<3>(0)
        .expand([batch, 6, 256]);
        let tokens = match sparse {
            Some(sparse) => Tensor::cat(vec![tokens, sparse], 1),
            None => tokens,
        };
        let mut queries = tokens.clone();
        let mut keys = (features[2].clone() + dense)
            .reshape([batch, 256, height * width])
            .swap_dims(1, 2);
        let position = position
            .reshape([1, 256, height * width])
            .swap_dims(1, 2)
            .expand([batch, height * width, 256]);
        for block in &self.blocks {
            (queries, keys) = block.forward(queries, keys, tokens.clone(), position.clone());
        }
        let attended = self.final_attention.forward(
            queries.clone() + tokens,
            keys.clone() + position,
            keys.clone(),
        );
        queries = self.final_norm.forward(queries + attended);
        let source = keys.swap_dims(1, 2).reshape([batch, 256, height, width]);
        let up = gelu(
            self.up_norm
                .forward(self.up1.forward(source) + self.high1.forward(features[1].clone())),
        );
        let up = gelu(self.up2.forward(up) + self.high0.forward(features[0].clone()));
        let hyper = Tensor::stack(
            self.hypernetworks
                .iter()
                .enumerate()
                .map(|(index, mlp)| {
                    mlp.forward(
                        queries
                            .clone()
                            .slice([0..batch, index + 2..index + 3, 0..256])
                            .reshape([batch, 256]),
                    )
                })
                .collect(),
            1,
        );
        Sam2Output {
            mask_logits: hyper
                .matmul(up.reshape([batch, 32, height * width * 16]))
                .reshape([batch, 4, height * 4, width * 4]),
            iou: sigmoid(
                self.iou_head.forward(
                    queries
                        .clone()
                        .slice([0..batch, 1..2, 0..256])
                        .reshape([batch, 256]),
                ),
            ),
            object_score_logits: self.object_head.forward(
                queries
                    .slice([0..batch, 0..1, 0..256])
                    .reshape([batch, 256]),
            ),
        }
    }
    pub(crate) fn import(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        let p = "sam_mask_decoder";
        self.object_token = Param::from_tensor(reader.tensor(
            &format!("{p}.obj_score_token.weight"),
            [1, 256],
            device,
        )?);
        self.iou_token = Param::from_tensor(reader.tensor(
            &format!("{p}.iou_token.weight"),
            [1, 256],
            device,
        )?);
        self.mask_tokens = Param::from_tensor(reader.tensor(
            &format!("{p}.mask_tokens.weight"),
            [4, 256],
            device,
        )?);
        self.blocks = self
            .blocks
            .into_iter()
            .enumerate()
            .map(|(i, block)| block.import(reader, &format!("{p}.transformer.layers.{i}"), device))
            .collect::<Result<_>>()?;
        self.final_attention = self.final_attention.import(
            reader,
            &format!("{p}.transformer.final_attn_token_to_image"),
            device,
        )?;
        self.final_norm = reader.layer_norm(
            self.final_norm,
            &format!("{p}.transformer.norm_final_attn"),
            device,
        )?;
        self.up1 = import_transpose(self.up1, reader, &format!("{p}.output_upscaling.0"), device)?;
        self.up_norm = self
            .up_norm
            .import(reader, &format!("{p}.output_upscaling.1"), device)?;
        self.up2 = import_transpose(self.up2, reader, &format!("{p}.output_upscaling.3"), device)?;
        self.high0 = reader.conv2d(self.high0, &format!("{p}.conv_s0"), device)?;
        self.high1 = reader.conv2d(self.high1, &format!("{p}.conv_s1"), device)?;
        self.hypernetworks = self
            .hypernetworks
            .into_iter()
            .enumerate()
            .map(|(i, mlp)| {
                mlp.import(
                    reader,
                    &format!("{p}.output_hypernetworks_mlps.{i}"),
                    device,
                )
            })
            .collect::<Result<_>>()?;
        self.iou_head =
            self.iou_head
                .import(reader, &format!("{p}.iou_prediction_head"), device)?;
        self.object_head =
            self.object_head
                .import(reader, &format!("{p}.pred_obj_score_head"), device)?;
        Ok(self)
    }
}
fn import_transpose(
    mut layer: ConvTranspose2d,
    reader: &mut SafeTensorReader<'_>,
    prefix: &str,
    device: &Device,
) -> Result<ConvTranspose2d> {
    layer.weight = Param::from_tensor(reader.tensor(
        &format!("{prefix}.weight"),
        layer.weight.dims(),
        device,
    )?);
    if let Some(bias) = layer.bias.take() {
        layer.bias = Some(Param::from_tensor(reader.tensor(
            &format!("{prefix}.bias"),
            bias.dims(),
            device,
        )?));
    }
    Ok(layer)
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;
    use crate::Sam2Point;
    #[test]
    fn sam2_prompt_fourier_coordinates_box_order_and_padding_match_image_predictor() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let device = Device::flex();
        let mut encoder = PromptEncoder::new(&device);
        let mut gaussian = vec![0.0; 256];
        gaussian[0] = 1.0;
        gaussian[129] = 1.0;
        encoder.gaussian = Param::from_tensor(Tensor::from_data(
            TensorData::new(gaussian, [2, 128]),
            &device,
        ));
        for (index, point) in encoder.points.iter_mut().enumerate() {
            *point = Param::from_tensor(Tensor::full([1, 256], (index + 1) as f32, &device));
        }
        encoder.not_a_point = Param::from_tensor(Tensor::full([1, 256], -7.0, &device));
        let prompt = Sam2Prompt {
            bbox: Some([100.0, 200.0, 800.0, 900.0]),
            points: vec![Sam2Point {
                x: 255.5,
                y: 511.5,
                label: 1,
            }],
            mask: None,
        };
        let (sparse, dense) = encoder.forward(&prompt, 4, &device);
        let sparse = sparse.unwrap();
        assert_eq!(sparse.dims(), [1, 4, 256]);
        assert_eq!(dense.dims(), [1, 256, 4, 4]);
        let values = sparse.into_data().try_to_vec::<f32>().unwrap();
        for (index, (x, y, embedding)) in [
            (100.0f32, 200.0f32, 3.0f32),
            (800.0, 900.0, 4.0),
            (255.5, 511.5, 2.0),
        ]
        .into_iter()
        .enumerate()
        {
            let phase_x = (((x + 0.5) / 1024.0) * 2.0 - 1.0) * 2.0 * std::f32::consts::PI;
            let phase_y = (((y + 0.5) / 1024.0) * 2.0 - 1.0) * 2.0 * std::f32::consts::PI;
            assert!((values[index * 256] - phase_x.sin() - embedding).abs() < 1e-6);
            assert!((values[index * 256 + 1] - phase_y.sin() - embedding).abs() < 1e-6);
            assert!((values[index * 256 + 128] - phase_x.cos() - embedding).abs() < 1e-6);
        }
        assert!(values[3 * 256..].iter().all(|value| *value == -7.0));
        assert!(
            encoder
                .forward(&Sam2Prompt::default(), 4, &device)
                .0
                .is_none()
        );
        let position = encoder
            .dense_position(4, &device)
            .into_data()
            .try_to_vec::<f32>()
            .unwrap();
        let expected = (((0.5f32 / 4.0) * 2.0 - 1.0) * 2.0 * std::f32::consts::PI).sin();
        assert!((position[0] - expected).abs() < 1e-6);
    }
}

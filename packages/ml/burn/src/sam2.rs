// Copyright (c) Meta Platforms, Inc. and affiliates. All rights reserved.
// Adapted from SAM 2.1 under Apache-2.0; see licenses/SAM2.
pub use crate::sam2_decoder::Sam2Output;
use crate::{
    Result, Sam2Config, Sam2Prompt,
    pretrained::SafeTensorReader,
    sam2_decoder::{MaskDecoder, PromptEncoder},
    sam2_hiera::HieraImageEncoder,
};
use burn::{
    module::{Module, Param},
    nn::Initializer,
    tensor::{
        Device, Tensor,
        activation::{log_sigmoid, sigmoid},
    },
};

#[derive(Module, Debug)]
pub struct Sam2 {
    image_encoder: HieraImageEncoder,
    prompt_encoder: PromptEncoder,
    mask_decoder: MaskDecoder,
    no_memory: Param<Tensor<3>>,
}
impl Sam2 {
    pub fn new(config: &Sam2Config, device: &Device) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            image_encoder: HieraImageEncoder::new(device),
            prompt_encoder: PromptEncoder::new(device),
            mask_decoder: MaskDecoder::new(device),
            no_memory: Initializer::Normal {
                mean: 0.0,
                std: 0.02,
            }
            .init([1, 1, 256], device),
        })
    }
    pub fn configure_fine_tuning(self, freeze_image_encoder: bool) -> Self {
        let mut model = self.unfreeze();
        if freeze_image_encoder {
            model.image_encoder = model.image_encoder.freeze();
            model.no_memory = model.no_memory.freeze();
        }
        model.train()
    }
    pub fn forward(&self, images: Tensor<4>, prompts: &[Sam2Prompt]) -> Result<Sam2Output> {
        Sam2Config {
            variant: crate::Sam2Variant::HieraTiny,
        }
        .validate_input_shape(&images.dims())?;
        if images.dims()[0] != prompts.len() {
            return Err(crate::Error::Invalid(
                "SAM needs one prompt per image".into(),
            ));
        }
        for prompt in prompts {
            prompt.validate()?;
        }
        Ok(self.forward_images(images, prompts))
    }
    pub(crate) fn forward_images(&self, images: Tensor<4>, prompts: &[Sam2Prompt]) -> Sam2Output {
        let device = images.device();
        let mut features = self.image_encoder.forward(images);
        features[2] = features[2].clone() + self.no_memory.val().reshape([1, 256, 1, 1]);
        let grid = features[2].dims()[2];
        let position = self.prompt_encoder.dense_position(grid, &device);
        let mut masks = Vec::with_capacity(prompts.len());
        let mut ious = Vec::with_capacity(prompts.len());
        let mut objects = Vec::with_capacity(prompts.len());
        for (index, prompt) in prompts.iter().enumerate() {
            let features = features
                .iter()
                .map(|feature| {
                    let [_, c, h, w] = feature.dims();
                    feature.clone().slice([index..index + 1, 0..c, 0..h, 0..w])
                })
                .collect::<Vec<_>>();
            let (sparse, dense) = self.prompt_encoder.forward(prompt, grid, &device);
            let output = self
                .mask_decoder
                .forward(&features, position.clone(), sparse, dense);
            masks.push(output.mask_logits);
            ious.push(output.iou);
            objects.push(output.object_score_logits);
        }
        Sam2Output {
            mask_logits: Tensor::cat(masks, 0),
            iou: Tensor::cat(ious, 0),
            object_score_logits: Tensor::cat(objects, 0),
        }
    }
    pub(crate) fn import(
        mut self,
        reader: &mut SafeTensorReader<'_>,
        device: &Device,
    ) -> Result<Self> {
        self.image_encoder = self.image_encoder.import(reader, device)?;
        self.prompt_encoder = self.prompt_encoder.import(reader, device)?;
        self.mask_decoder = self.mask_decoder.import(reader, device)?;
        self.no_memory = Param::from_tensor(reader.tensor("no_mem_embed", [1, 1, 256], device)?);
        Ok(self)
    }
}

impl Sam2Output {
    /// Supervise all mask proposals, with focal and Dice losses plus quality and presence heads.
    pub fn loss(&self, target: Tensor<4>) -> Tensor<1> {
        let [batch, _, height, width] = self.mask_logits.dims();
        let target = target.expand([batch, 4, height, width]);
        let probability = sigmoid(self.mask_logits.clone());
        let positive = target.clone() * 0.25;
        let negative = (target.clone() * -1.0 + 1.0) * 0.75;
        let pt = target.clone() * probability.clone()
            + (target.clone() * -1.0 + 1.0) * (probability.clone() * -1.0 + 1.0);
        let bce = -log_sigmoid(self.mask_logits.clone()) * target.clone()
            - log_sigmoid(-self.mask_logits.clone()) * (-target.clone() + 1.0);
        let focal = ((positive + negative) * (pt * -1.0 + 1.0).powf_scalar(2.0) * bce).mean();
        let flatten = |x: Tensor<4>| x.reshape([batch, 4, height * width]);
        let intersection = flatten(probability.clone() * target.clone()).sum_dim(2);
        let total = flatten(probability).sum_dim(2) + flatten(target.clone()).sum_dim(2);
        let dice = ((intersection * 2.0 + 1.0) / (total + 1.0) * -1.0 + 1.0).mean();
        let predicted = self.mask_logits.clone().detach().greater_elem(0.0).float();
        let true_intersection = flatten(predicted.clone() * target.clone()).sum_dim(2);
        let union =
            flatten(predicted.clone() + target.clone() - predicted * target.clone()).sum_dim(2);
        let true_iou = (true_intersection / union.clamp_min(1.0)).reshape([batch, 4]);
        let iou = (self.iou.clone() - true_iou).powf_scalar(2.0).mean();
        let present = target
            .slice([0..batch, 0..1, 0..height, 0..width])
            .reshape([batch, height * width])
            .sum_dim(1)
            .greater_elem(0.0)
            .float();
        let object = self.object_score_logits.clone();
        let object_loss = (-log_sigmoid(object.clone()) * present.clone()
            - log_sigmoid(-object) * (-present + 1.0))
            .mean();
        focal * 20.0 + dice + iou + object_loss
    }
}

#[cfg(all(test, feature = "cpu"))]
mod tests {
    use super::*;
    use crate::{Sam2Point, Sam2Variant};
    use burn::{
        module::{ModuleVisitor, Param},
        optim::{AdamConfig, GradientsParams},
        tensor::TensorData,
    };
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;

    fn parameters(model: &Sam2) -> BTreeMap<String, String> {
        #[derive(Default)]
        struct Values {
            path: Vec<String>,
            values: BTreeMap<String, String>,
        }
        impl ModuleVisitor for Values {
            fn enter_module(&mut self, name: &str, _: &str) {
                self.path.push(name.into());
            }
            fn exit_module(&mut self, _: &str, _: &str) {
                self.path.pop();
            }
            fn visit_float<const D: usize>(&mut self, param: &Param<Tensor<D>>) {
                let mut hash = Sha256::new();
                for value in param.val().into_data().iter::<f32>() {
                    hash.update(value.to_le_bytes());
                }
                self.values
                    .insert(self.path.join("."), format!("{:x}", hash.finalize()));
            }
        }
        let mut visitor = Values::default();
        model.visit(&mut visitor);
        assert!(!visitor.values.is_empty());
        visitor.values
    }

    #[test]
    fn sam2_prompt_validation_and_model_geometry_are_explicit() {
        let config = Sam2Config {
            variant: Sam2Variant::HieraTiny,
        };
        assert!(config.validate_input_shape(&[1, 3, 1024, 1024]).is_ok());
        assert!(config.validate_input_shape(&[1, 3, 224, 224]).is_err());
        let mut prompt = Sam2Prompt {
            points: vec![Sam2Point {
                x: 22.0,
                y: 50.0,
                label: 1,
            }],
            ..Default::default()
        };
        assert!(prompt.validate().is_ok());
        prompt.points[0].label = 2;
        assert!(prompt.validate().is_err());
        prompt.points.clear();
        prompt.bbox = Some([30.0, 10.0, 20.0, 70.0]);
        assert!(prompt.validate().is_err());
    }

    #[test]
    fn sam2_zero_logits_have_correct_focal_dice_and_presence_gradients() {
        let device = Device::flex().autodiff();
        let mask_logits = Tensor::<4>::zeros([2, 4, 1, 1], &device).require_grad();
        let object_score_logits = Tensor::<2>::zeros([2, 1], &device).require_grad();
        let output = Sam2Output {
            mask_logits: mask_logits.clone(),
            iou: Tensor::zeros([2, 4], &device),
            object_score_logits: object_score_logits.clone(),
        };
        let target =
            Tensor::<4>::from_data(TensorData::new(vec![0.0f32, 1.0], [2, 1, 1, 1]), &device);
        let gradients = output.loss(target).backward();
        let mask = mask_logits
            .grad(&gradients)
            .unwrap()
            .into_data()
            .try_to_vec::<f32>()
            .unwrap();
        // At p=0.5, d(focal)/dx = +/- alpha * (1/8 + ln(2)/4).
        // Dice contributes 1/9 for an empty target and -3/25 for foreground.
        let focal_slope = 0.125 + std::f32::consts::LN_2 * 0.25;
        let expected = [
            (20.0 * 0.75 * focal_slope + 1.0 / 9.0) / 8.0,
            (-20.0 * 0.25 * focal_slope - 3.0 / 25.0) / 8.0,
        ];
        for (index, actual) in mask.into_iter().enumerate() {
            assert!(
                (actual - expected[index / 4]).abs() < 1e-6,
                "mask gradient {index}: {actual}, expected {}",
                expected[index / 4]
            );
        }
        let object = object_score_logits
            .grad(&gradients)
            .unwrap()
            .into_data()
            .try_to_vec::<f32>()
            .unwrap();
        for (actual, expected) in object.into_iter().zip([0.25, -0.25]) {
            assert!(
                (actual - expected).abs() < 1e-6,
                "presence gradient {actual}, expected {expected}"
            );
        }
    }

    #[test]
    fn sam2_native_mask_decoder_trains_with_frozen_hiera_and_resumes_optimizer() {
        let _lock = crate::tests::TEST_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let device = Device::flex().autodiff();
        device.seed(394);
        let config = Sam2Config {
            variant: Sam2Variant::HieraTiny,
        };
        let mut model = Sam2::new(&config, &device)
            .unwrap()
            .configure_fine_tuning(true);
        let input = Tensor::from_data(
            TensorData::new(
                (0..3 * 64 * 64)
                    .map(|i| ((i as f32) * 0.001).sin())
                    .collect::<Vec<_>>(),
                [1, 3, 64, 64],
            ),
            &device,
        );
        let prompt = vec![Sam2Prompt {
            points: vec![Sam2Point {
                x: 340.0,
                y: 440.0,
                label: 1,
            }],
            bbox: Some([120.0, 80.0, 850.0, 960.0]),
            mask: None,
        }];
        let target = Tensor::from_data(
            TensorData::new(
                (0..16 * 16)
                    .map(|i| {
                        if i % 16 > 3 && i % 16 < 12 && i / 16 > 3 && i / 16 < 12 {
                            1.0
                        } else {
                            0.0
                        }
                    })
                    .collect::<Vec<_>>(),
                [1, 1, 16, 16],
            ),
            &device,
        );
        let before = parameters(&model);
        let output = model.forward_images(input.clone(), &prompt);
        assert_eq!(output.mask_logits.dims(), [1, 4, 16, 16]);
        assert_eq!(output.iou.dims(), [1, 4]);
        assert_eq!(output.object_score_logits.dims(), [1, 1]);
        let mut optimizer = AdamConfig::new().init();
        let loss = output.loss(target.clone());
        assert!(loss.clone().into_scalar::<f32>().is_finite());
        let grads = GradientsParams::from_grads(loss.backward(), &model);
        model = optimizer.step(1e-4, model, grads);
        let after = parameters(&model);
        let mut encoder_count = 0;
        let mut decoder_changed = false;
        for (name, value) in &before {
            if name.contains("image_encoder") || name.contains("no_memory") {
                assert_eq!(&after[name], value, "Frozen parameter changed: {name}");
                encoder_count += 1;
            }
            if name.contains("mask_decoder") && value != &after[name] {
                decoder_changed = true;
            }
        }
        assert!(encoder_count > 100);
        assert!(decoder_changed);
        let directory = tempfile::tempdir().unwrap();
        model
            .clone()
            .save_file(directory.path().join("model.bpk"))
            .unwrap();
        optimizer
            .save(directory.path().join("optimizer.bpk"))
            .unwrap();
        let mut resumed = Sam2::new(&config, &device)
            .unwrap()
            .try_load_file(directory.path().join("model.bpk"))
            .unwrap()
            .configure_fine_tuning(true);
        let mut resumed_optimizer = AdamConfig::new()
            .init()
            .load(directory.path().join("optimizer.bpk"))
            .unwrap();
        let output = model.forward_images(input.clone(), &prompt);
        let expected = output
            .mask_logits
            .clone()
            .into_data()
            .try_to_vec::<f32>()
            .unwrap();
        assert_eq!(
            resumed
                .forward_images(input.clone(), &prompt)
                .mask_logits
                .into_data()
                .try_to_vec::<f32>()
                .unwrap(),
            expected
        );
        let loss = output.loss(target.clone());
        let grads = GradientsParams::from_grads(loss.backward(), &model);
        model = optimizer.step(1e-4, model, grads);
        let loss = resumed.forward_images(input, &prompt).loss(target);
        let grads = GradientsParams::from_grads(loss.backward(), &resumed);
        resumed = resumed_optimizer.step(1e-4, resumed, grads);
        assert_eq!(parameters(&model), parameters(&resumed));
    }
}

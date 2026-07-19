/// # ONNX Object Detection Nodes
use crate::onnx::NodeOnnxSession;
#[cfg(feature = "execute")]
use crate::onnx::Provider;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::{BoundingBox, NodeImage};
#[cfg(feature = "execute")]
use flow_like_model_provider::ml::{
    ndarray::{Array2, Array3, Array4, ArrayView1, Axis, s},
    ort::{
        inputs,
        session::{Session, SessionInputValue, SessionOutputs},
        value::{DynValue, Value},
    },
};
#[cfg(feature = "execute")]
use flow_like_types::{
    Error,
    image::{DynamicImage, GenericImageView, Rgb, RgbImage, imageops::FilterType},
};
use flow_like_types::{Result, anyhow, async_trait, json::json};
#[cfg(feature = "execute")]
use std::borrow::Cow;
#[cfg(feature = "execute")]
use std::cmp::Ordering;

#[cfg(feature = "execute")]
// ## Object Detection Trait for Common Behavior
pub trait ObjectDetection {
    // Preprocessing
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error>;
    // Postprocessing
    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error>;
    // End-to-End Inference
    fn run(
        &self,
        session: &mut Session,
        img: &DynamicImage,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error>;
}

#[cfg(feature = "execute")]
#[derive(Clone, Copy)]
pub enum BoxLabelsScoresPreprocessing {
    DetectronBgrChw,
    ImagenetNchw,
}

#[cfg(feature = "execute")]
#[derive(Clone, Copy)]
pub enum YoloImageShapeKind {
    F32,
    I32,
    I64,
}

// ## Implementation for D-FINE Models
#[derive(Clone)]
pub struct DfineLike {
    pub input_width: u32,
    pub input_height: u32,
}

#[cfg(feature = "execute")]
impl ObjectDetection for DfineLike {
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error> {
        let (img_width, img_height) = (img.width() as i64, img.height() as i64);
        let images = img_to_arr(img, self.input_width, self.input_height)?;
        let orig_target_size = Array2::from_shape_vec((1, 2), vec![img_width, img_height])?;
        let images_data = Value::from_array(images)?;
        let orig_target_size_data = Value::from_array(orig_target_size)?;
        let session_inputs = inputs![
            "images" => images_data,
            "orig_target_sizes" => orig_target_size_data
        ];
        Ok(session_inputs)
    }

    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        conf_thres: f32,
        _iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let labels = outputs["labels"].try_extract_array::<i64>()?;
        let boxes = outputs["boxes"].try_extract_array::<f32>()?;
        let scores = outputs["scores"].try_extract_array::<f32>()?;
        let mut bboxes: Vec<BoundingBox> = boxes
            .axis_iter(Axis(1))
            .enumerate()
            .map(|(i, bbox)| {
                let bbox_xyxy = bbox.slice(s![0, ..]).to_vec();
                let (x1, y1, x2, y2) = (bbox_xyxy[0], bbox_xyxy[1], bbox_xyxy[2], bbox_xyxy[3]);
                let class_idx = labels.slice(s![.., i]).to_vec()[0];
                let score = scores.slice(s![.., i]).to_vec()[0];
                BoundingBox {
                    class_idx: class_idx as i32,
                    score,
                    x1,
                    y1,
                    x2,
                    y2,
                    class_name: coco_class_name(class_idx as i32),
                }
            })
            .filter(|b| b.score > conf_thres)
            .collect();
        bboxes.sort_unstable_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        bboxes.truncate(max_detect);
        Ok(bboxes)
    }

    fn run(
        &self,
        session: &mut Session,
        img: &DynamicImage,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let session_inputs = self.make_inputs(img)?;
        let session_outputs = session.run(session_inputs)?;
        let bboxes = self.make_results(session_outputs, conf_thres, iou_thres, max_detect)?;
        Ok(bboxes)
    }
}

// ## Implementation for YOLO Models
#[derive(Clone)]
pub struct YoloLike {
    pub input_width: u32,
    pub input_height: u32,
}

#[cfg(feature = "execute")]
impl ObjectDetection for YoloLike {
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error> {
        let images = img_to_arr(img, self.input_width, self.input_height)?;
        let images_data = Value::from_array(images)?;
        let session_inputs = inputs! [
            "images" => images_data,
        ];
        Ok(session_inputs)
    }

    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let output = outputs["output0"].try_extract_array::<f32>()?;
        let view_candidates = output.slice(s![0, 4.., ..]);
        let mask_candidates: Vec<bool> = view_candidates
            .axis_iter(Axis(1))
            .map(|col| col.iter().cloned().fold(f32::NEG_INFINITY, f32::max) > conf_thres)
            .collect();
        let idx_candidates: Vec<usize> = mask_candidates
            .iter()
            .enumerate()
            .filter_map(|(i, &keep)| if keep { Some(i) } else { None })
            .collect();
        let candidates_image = output.select(Axis(2), &idx_candidates).squeeze();
        let mut bboxes: Vec<BoundingBox> = Vec::with_capacity(candidates_image.len_of(Axis(1)));
        for candidate in candidates_image.axis_iter(Axis(1)) {
            let bbox = bounding_box_from_array(candidate.to_shape(candidate.len()).unwrap().view());
            bboxes.push(bbox);
        }
        let mut bboxes = nms(&bboxes, iou_thres);
        bboxes.truncate(max_detect); // keep only max detections
        Ok(bboxes)
    }

    fn run(
        &self,
        session: &mut Session,
        img: &DynamicImage,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let session_inputs = self.make_inputs(img)?;
        let session_outputs = session.run(session_inputs)?;
        let mut bboxes = self.make_results(session_outputs, conf_thres, iou_thres, max_detect)?;
        let (target_w, target_h) = (img.width() as f32, img.height() as f32);
        let scale_w = target_w / self.input_width as f32;
        let scale_h = target_h / self.input_height as f32;
        for bbox in &mut bboxes {
            bbox.scale(scale_w, scale_h);
        }
        Ok(bboxes)
    }
}

#[derive(Clone)]
pub struct BoxLabelsScoresLike {
    pub input_name: String,
    pub boxes_output_name: String,
    pub labels_output_name: String,
    pub scores_output_name: String,
    pub input_width: u32,
    pub input_height: u32,
    #[cfg(feature = "execute")]
    pub preprocessing: BoxLabelsScoresPreprocessing,
}

#[cfg(feature = "execute")]
impl ObjectDetection for BoxLabelsScoresLike {
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error> {
        match self.preprocessing {
            BoxLabelsScoresPreprocessing::DetectronBgrChw => {
                let image = img_to_chw_bgr_detectron(img)?;
                let image_data = Value::from_array(image)?;
                Ok(inputs![self.input_name.as_str() => image_data])
            }
            BoxLabelsScoresPreprocessing::ImagenetNchw => {
                let image = img_to_arr_nchw_imagenet(img, self.input_width, self.input_height)?;
                let image_data = Value::from_array(image)?;
                Ok(inputs![self.input_name.as_str() => image_data])
            }
        }
    }

    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        conf_thres: f32,
        _iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let boxes = outputs[self.boxes_output_name.as_str()].try_extract_array::<f32>()?;
        let scores = outputs[self.scores_output_name.as_str()].try_extract_array::<f32>()?;
        let labels = extract_i32_vec(&outputs[self.labels_output_name.as_str()])?;
        let boxes = boxes
            .as_slice()
            .ok_or_else(|| anyhow!("boxes output is not contiguous"))?;
        let scores: Vec<f32> = scores.iter().copied().collect();
        let count = (boxes.len() / 4).min(scores.len()).min(labels.len());

        let mut bboxes = Vec::with_capacity(count);
        for i in 0..count {
            let score = scores[i];
            if score <= conf_thres {
                continue;
            }

            let base = i * 4;
            bboxes.push(BoundingBox {
                class_idx: labels[i],
                score,
                x1: boxes[base],
                y1: boxes[base + 1],
                x2: boxes[base + 2],
                y2: boxes[base + 3],
                class_name: coco_class_name_with_background(labels[i]),
            });
        }

        bboxes.sort_unstable_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        bboxes.truncate(max_detect);
        Ok(bboxes)
    }

    fn run(
        &self,
        session: &mut Session,
        img: &DynamicImage,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let session_inputs = self.make_inputs(img)?;
        let session_outputs = session.run(session_inputs)?;
        let mut bboxes = self.make_results(session_outputs, conf_thres, iou_thres, max_detect)?;
        if matches!(
            self.preprocessing,
            BoxLabelsScoresPreprocessing::ImagenetNchw
        ) {
            let scale_w = img.width() as f32 / self.input_width as f32;
            let scale_h = img.height() as f32 / self.input_height as f32;
            for bbox in &mut bboxes {
                bbox.scale(scale_w, scale_h);
            }
        } else {
            let ratio = detectron_resize_ratio(img);
            for bbox in &mut bboxes {
                bbox.scale(1.0 / ratio, 1.0 / ratio);
            }
        }
        Ok(bboxes)
    }
}

#[derive(Clone)]
pub struct SsdMobileNetLike {
    pub input_name: String,
    pub num_detections_output_name: String,
    pub boxes_output_name: String,
    pub scores_output_name: String,
    pub classes_output_name: String,
}

#[cfg(feature = "execute")]
impl ObjectDetection for SsdMobileNetLike {
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error> {
        let image = img_to_arr_nhwc_u8(img, img.width(), img.height())?;
        let image_data = Value::from_array(image)?;
        Ok(inputs![self.input_name.as_str() => image_data])
    }

    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        conf_thres: f32,
        _iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let num_detections = extract_f32_vec(&outputs[self.num_detections_output_name.as_str()])?
            .first()
            .copied()
            .unwrap_or(0.0)
            .max(0.0) as usize;
        let boxes = outputs[self.boxes_output_name.as_str()].try_extract_array::<f32>()?;
        let scores = outputs[self.scores_output_name.as_str()].try_extract_array::<f32>()?;
        let classes = extract_i32_vec(&outputs[self.classes_output_name.as_str()])?;
        let boxes = boxes
            .as_slice()
            .ok_or_else(|| anyhow!("detection_boxes output is not contiguous"))?;
        let scores: Vec<f32> = scores.iter().copied().collect();
        let count = num_detections
            .min(boxes.len() / 4)
            .min(scores.len())
            .min(classes.len());

        let mut bboxes = Vec::with_capacity(count);
        for i in 0..count {
            let score = scores[i];
            if score <= conf_thres {
                continue;
            }

            let base = i * 4;
            bboxes.push(BoundingBox {
                class_idx: classes[i],
                score,
                x1: boxes[base + 1],
                y1: boxes[base],
                x2: boxes[base + 3],
                y2: boxes[base + 2],
                class_name: coco_category_id_class_name(classes[i]),
            });
        }

        bboxes.sort_unstable_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        bboxes.truncate(max_detect);
        Ok(bboxes)
    }

    fn run(
        &self,
        session: &mut Session,
        img: &DynamicImage,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let session_inputs = self.make_inputs(img)?;
        let session_outputs = session.run(session_inputs)?;
        let mut bboxes = self.make_results(session_outputs, conf_thres, iou_thres, max_detect)?;
        let (w, h) = (img.width() as f32, img.height() as f32);
        for bbox in &mut bboxes {
            bbox.x1 *= w;
            bbox.x2 *= w;
            bbox.y1 *= h;
            bbox.y2 *= h;
        }
        Ok(bboxes)
    }
}

#[derive(Clone)]
pub struct YoloV3Like {
    pub image_input_name: String,
    pub image_shape_input_name: String,
    pub boxes_output_name: String,
    pub scores_output_name: String,
    pub indices_output_name: String,
    pub input_width: u32,
    pub input_height: u32,
    #[cfg(feature = "execute")]
    pub image_shape_kind: YoloImageShapeKind,
}

#[cfg(feature = "execute")]
impl ObjectDetection for YoloV3Like {
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error> {
        let image = img_to_arr_letterbox_nchw(img, self.input_width, self.input_height)?;
        let image_data = Value::from_array(image)?;
        match self.image_shape_kind {
            YoloImageShapeKind::F32 => {
                let shape =
                    Array2::from_shape_vec((1, 2), vec![img.height() as f32, img.width() as f32])?;
                let shape_data = Value::from_array(shape)?;
                Ok(inputs![
                    self.image_input_name.as_str() => image_data,
                    self.image_shape_input_name.as_str() => shape_data
                ])
            }
            YoloImageShapeKind::I32 => {
                let shape =
                    Array2::from_shape_vec((1, 2), vec![img.height() as i32, img.width() as i32])?;
                let shape_data = Value::from_array(shape)?;
                Ok(inputs![
                    self.image_input_name.as_str() => image_data,
                    self.image_shape_input_name.as_str() => shape_data
                ])
            }
            YoloImageShapeKind::I64 => {
                let shape =
                    Array2::from_shape_vec((1, 2), vec![img.height() as i64, img.width() as i64])?;
                let shape_data = Value::from_array(shape)?;
                Ok(inputs![
                    self.image_input_name.as_str() => image_data,
                    self.image_shape_input_name.as_str() => shape_data
                ])
            }
        }
    }

    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        conf_thres: f32,
        _iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let boxes = outputs[self.boxes_output_name.as_str()].try_extract_array::<f32>()?;
        let scores = outputs[self.scores_output_name.as_str()].try_extract_array::<f32>()?;
        let indices = extract_i64_vec(&outputs[self.indices_output_name.as_str()])?;
        let boxes_shape = boxes.shape();
        let scores_shape = scores.shape();
        if boxes_shape.len() != 3 || scores_shape.len() != 3 {
            return Err(anyhow!("YOLOv3 outputs have unexpected rank"));
        }

        let boxes_batches = boxes_shape[0];
        let scores_batches = scores_shape[0];
        let boxes_per_batch = boxes_shape[1];
        let classes_per_batch = scores_shape[1];
        let score_boxes = scores_shape[2];
        let boxes = boxes
            .as_slice()
            .ok_or_else(|| anyhow!("boxes output is not contiguous"))?;
        let scores = scores
            .as_slice()
            .ok_or_else(|| anyhow!("scores output is not contiguous"))?;

        yolo_v3_selected_boxes(
            boxes,
            boxes_batches,
            boxes_per_batch,
            scores,
            scores_batches,
            classes_per_batch,
            score_boxes,
            &indices,
            conf_thres,
            max_detect,
        )
    }

    fn run(
        &self,
        session: &mut Session,
        img: &DynamicImage,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let session_inputs = self.make_inputs(img)?;
        let session_outputs = session.run(session_inputs)?;
        self.make_results(session_outputs, conf_thres, iou_thres, max_detect)
    }
}

#[derive(Clone)]
pub struct YoloV2GridLike {
    pub input_name: String,
    pub output_name: String,
    pub input_width: u32,
    pub input_height: u32,
    pub num_classes: usize,
}

#[cfg(feature = "execute")]
impl ObjectDetection for YoloV2GridLike {
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error> {
        let image = img_to_arr(img, self.input_width, self.input_height)?;
        let image_data = Value::from_array(image)?;
        Ok(inputs![self.input_name.as_str() => image_data])
    }

    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let output = outputs[self.output_name.as_str()].try_extract_array::<f32>()?;
        let shape = output.shape();
        if shape.len() != 4 {
            return Err(anyhow!("YOLOv2 grid output has unexpected rank"));
        }

        let channels = shape[1];
        let grid_h = shape[2];
        let grid_w = shape[3];
        let attrs = self.num_classes + 5;
        let anchors = channels / attrs;
        if anchors == 0 || channels % attrs != 0 {
            return Err(anyhow!("YOLOv2 grid output has unexpected channel count"));
        }

        let output = output
            .as_slice()
            .ok_or_else(|| anyhow!("YOLOv2 grid output is not contiguous"))?;
        let anchor_dims = yolo_v2_anchors(self.num_classes);
        let mut bboxes = Vec::new();
        for anchor in 0..anchors.min(anchor_dims.len()) {
            let (anchor_w, anchor_h) = anchor_dims[anchor];
            for gy in 0..grid_h {
                for gx in 0..grid_w {
                    let base_channel = anchor * attrs;
                    let tx = yolo_grid_value(output, base_channel, gy, gx, grid_h, grid_w);
                    let ty = yolo_grid_value(output, base_channel + 1, gy, gx, grid_h, grid_w);
                    let tw = yolo_grid_value(output, base_channel + 2, gy, gx, grid_h, grid_w);
                    let th = yolo_grid_value(output, base_channel + 3, gy, gx, grid_h, grid_w);
                    let objectness = sigmoid(yolo_grid_value(
                        output,
                        base_channel + 4,
                        gy,
                        gx,
                        grid_h,
                        grid_w,
                    ));

                    let mut best_class = 0usize;
                    let mut best_logit = f32::NEG_INFINITY;
                    for class_idx in 0..self.num_classes {
                        let logit = yolo_grid_value(
                            output,
                            base_channel + 5 + class_idx,
                            gy,
                            gx,
                            grid_h,
                            grid_w,
                        );
                        if logit > best_logit {
                            best_logit = logit;
                            best_class = class_idx;
                        }
                    }

                    let mut class_exp_sum = 0.0_f32;
                    for class_idx in 0..self.num_classes {
                        let logit = yolo_grid_value(
                            output,
                            base_channel + 5 + class_idx,
                            gy,
                            gx,
                            grid_h,
                            grid_w,
                        );
                        class_exp_sum += (logit - best_logit).exp();
                    }
                    let class_prob = if class_exp_sum > 0.0 {
                        1.0 / class_exp_sum
                    } else {
                        0.0
                    };

                    let score = objectness * class_prob;
                    if score <= conf_thres {
                        continue;
                    }

                    let center_x =
                        (sigmoid(tx) + gx as f32) * self.input_width as f32 / grid_w as f32;
                    let center_y =
                        (sigmoid(ty) + gy as f32) * self.input_height as f32 / grid_h as f32;
                    let width = tw.exp() * anchor_w * self.input_width as f32 / grid_w as f32;
                    let height = th.exp() * anchor_h * self.input_height as f32 / grid_h as f32;
                    let (x1, y1, x2, y2) = xywh_to_xyxy(&center_x, &center_y, &width, &height);

                    bboxes.push(BoundingBox {
                        class_idx: best_class as i32,
                        score,
                        x1,
                        y1,
                        x2,
                        y2,
                        class_name: class_name_for_contiguous_label(
                            best_class as i32,
                            self.num_classes,
                        ),
                    });
                }
            }
        }

        let mut bboxes = nms(&bboxes, iou_thres);
        bboxes.truncate(max_detect);
        Ok(bboxes)
    }

    fn run(
        &self,
        session: &mut Session,
        img: &DynamicImage,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let session_inputs = self.make_inputs(img)?;
        let session_outputs = session.run(session_inputs)?;
        let mut bboxes = self.make_results(session_outputs, conf_thres, iou_thres, max_detect)?;
        let scale_w = img.width() as f32 / self.input_width as f32;
        let scale_h = img.height() as f32 / self.input_height as f32;
        for bbox in &mut bboxes {
            bbox.scale(scale_w, scale_h);
        }
        Ok(bboxes)
    }
}

#[derive(Clone)]
pub struct YoloV4Like {
    pub input_name: String,
    pub input_width: u32,
    pub input_height: u32,
}

#[cfg(feature = "execute")]
impl ObjectDetection for YoloV4Like {
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error> {
        let image = img_to_arr_letterbox_nhwc_f32(img, self.input_width, self.input_height)?;
        let image_data = Value::from_array(image)?;
        Ok(inputs![self.input_name.as_str() => image_data])
    }

    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let mut bboxes = Vec::new();
        for output_name in outputs.keys() {
            let output = outputs[output_name].try_extract_array::<f32>()?;
            let shape = output.shape();
            if shape.len() != 5 || *shape.last().unwrap_or(&0) < 6 {
                continue;
            }

            let grid_h = shape[1];
            let grid_w = shape[2];
            let anchors = shape[3];
            let attrs = shape[4];
            let stride = self.input_width as f32 / grid_w as f32;
            let anchor_dims = yolo_v4_anchors(grid_w);
            let xyscale = yolo_v4_xyscale(grid_w);
            let output = output
                .as_slice()
                .ok_or_else(|| anyhow!("YOLOv4 output is not contiguous"))?;

            for gy in 0..grid_h {
                for gx in 0..grid_w {
                    for anchor in 0..anchors.min(anchor_dims.len()) {
                        let base = (((gy * grid_w + gx) * anchors + anchor) * attrs) as usize;
                        let objectness = sigmoid(output[base + 4]);
                        let mut best_class = 0usize;
                        let mut best_score = f32::NEG_INFINITY;
                        for class_idx in 0..(attrs - 5) {
                            let class_score = sigmoid(output[base + 5 + class_idx]);
                            if class_score > best_score {
                                best_score = class_score;
                                best_class = class_idx;
                            }
                        }

                        let score = objectness * best_score;
                        if score <= conf_thres {
                            continue;
                        }

                        let (anchor_w, anchor_h) = anchor_dims[anchor];
                        let center_x = ((sigmoid(output[base]) * xyscale) - 0.5 * (xyscale - 1.0)
                            + gx as f32)
                            * stride;
                        let center_y = ((sigmoid(output[base + 1]) * xyscale)
                            - 0.5 * (xyscale - 1.0)
                            + gy as f32)
                            * stride;
                        let width = output[base + 2].exp() * anchor_w;
                        let height = output[base + 3].exp() * anchor_h;
                        let (x1, y1, x2, y2) = xywh_to_xyxy(&center_x, &center_y, &width, &height);
                        bboxes.push(BoundingBox {
                            class_idx: best_class as i32,
                            score,
                            x1,
                            y1,
                            x2,
                            y2,
                            class_name: coco_class_name(best_class as i32),
                        });
                    }
                }
            }
        }

        let mut bboxes = nms(&bboxes, iou_thres);
        bboxes.truncate(max_detect);
        Ok(bboxes)
    }

    fn run(
        &self,
        session: &mut Session,
        img: &DynamicImage,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let session_inputs = self.make_inputs(img)?;
        let session_outputs = session.run(session_inputs)?;
        let mut bboxes = self.make_results(session_outputs, conf_thres, iou_thres, max_detect)?;
        unletterbox_boxes(
            &mut bboxes,
            img.width(),
            img.height(),
            self.input_width,
            self.input_height,
        );
        Ok(bboxes)
    }
}

#[derive(Clone)]
pub struct RetinaNetLike {
    pub input_name: String,
    pub output_names: Vec<String>,
    pub input_width: u32,
    pub input_height: u32,
    pub resize_input: bool,
}

#[cfg(feature = "execute")]
impl ObjectDetection for RetinaNetLike {
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error> {
        let (input_width, input_height) = self.input_size_for_image(img);
        let image = img_to_arr_nchw_imagenet(img, input_width, input_height)?;
        let image_data = Value::from_array(image)?;
        Ok(inputs![self.input_name.as_str() => image_data])
    }

    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        self.make_results_for_input(
            outputs,
            conf_thres,
            iou_thres,
            max_detect,
            self.input_width.max(1),
            self.input_height.max(1),
        )
    }

    fn run(
        &self,
        session: &mut Session,
        img: &DynamicImage,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
    ) -> Result<Vec<BoundingBox>, Error> {
        let (input_width, input_height) = self.input_size_for_image(img);
        let session_inputs = self.make_inputs(img)?;
        let session_outputs = session.run(session_inputs)?;
        let mut bboxes = self.make_results_for_input(
            session_outputs,
            conf_thres,
            iou_thres,
            max_detect,
            input_width,
            input_height,
        )?;
        if self.resize_input {
            let scale_w = img.width() as f32 / input_width as f32;
            let scale_h = img.height() as f32 / input_height as f32;
            for bbox in &mut bboxes {
                bbox.scale(scale_w, scale_h);
            }
        }
        Ok(bboxes)
    }
}

#[cfg(feature = "execute")]
impl RetinaNetLike {
    fn input_size_for_image(&self, img: &DynamicImage) -> (u32, u32) {
        if self.resize_input {
            (self.input_width, self.input_height)
        } else {
            (img.width(), img.height())
        }
    }

    fn make_results_for_input(
        &self,
        outputs: SessionOutputs<'_>,
        conf_thres: f32,
        iou_thres: f32,
        max_detect: usize,
        input_width: u32,
        _input_height: u32,
    ) -> Result<Vec<BoundingBox>, Error> {
        let mut bboxes = Vec::new();
        for cls_name in &self.output_names {
            let cls_output = outputs[cls_name.as_str()].try_extract_array::<f32>()?;
            let cls_shape = cls_output.shape();
            if cls_shape.len() != 4 || cls_shape[1] % 80 != 0 {
                continue;
            }

            let grid_h = cls_shape[2];
            let grid_w = cls_shape[3];
            let Some(box_name) = self.output_names.iter().find(|name| {
                if *name == cls_name {
                    return false;
                }

                outputs
                    .get(name.as_str())
                    .and_then(|value| value.try_extract_array::<f32>().ok())
                    .map(|arr| {
                        let shape = arr.shape();
                        shape.len() == 4
                            && shape[1] % 4 == 0
                            && shape[1] % 80 != 0
                            && shape[2] == grid_h
                            && shape[3] == grid_w
                    })
                    .unwrap_or(false)
            }) else {
                continue;
            };

            let box_output = outputs[box_name.as_str()].try_extract_array::<f32>()?;
            let cls = cls_output
                .as_slice()
                .ok_or_else(|| anyhow!("RetinaNet class output is not contiguous"))?;
            let boxes = box_output
                .as_slice()
                .ok_or_else(|| anyhow!("RetinaNet box output is not contiguous"))?;
            let anchors = cls_shape[1] / 80;
            let stride = input_width as f32 / grid_w as f32;
            let anchor_dims = retinanet_anchors(stride);

            for gy in 0..grid_h {
                for gx in 0..grid_w {
                    for anchor in 0..anchors.min(anchor_dims.len()) {
                        let mut best_class = 0usize;
                        let mut best_score = f32::NEG_INFINITY;
                        for class_idx in 0..80usize {
                            let channel = anchor * 80 + class_idx;
                            let logit = nchw_value(cls, channel, gy, gx, grid_h, grid_w);
                            let score = sigmoid(logit);
                            if score > best_score {
                                best_score = score;
                                best_class = class_idx;
                            }
                        }

                        if best_score <= conf_thres {
                            continue;
                        }

                        let box_channel = anchor * 4;
                        let dx = nchw_value(boxes, box_channel, gy, gx, grid_h, grid_w);
                        let dy = nchw_value(boxes, box_channel + 1, gy, gx, grid_h, grid_w);
                        let dw = nchw_value(boxes, box_channel + 2, gy, gx, grid_h, grid_w);
                        let dh = nchw_value(boxes, box_channel + 3, gy, gx, grid_h, grid_w);
                        let (anchor_w, anchor_h) = anchor_dims[anchor];
                        let anchor_cx = (gx as f32 + 0.5) * stride;
                        let anchor_cy = (gy as f32 + 0.5) * stride;
                        let center_x = dx * anchor_w + anchor_cx;
                        let center_y = dy * anchor_h + anchor_cy;
                        let width = dw.exp() * anchor_w;
                        let height = dh.exp() * anchor_h;
                        let (x1, y1, x2, y2) = xywh_to_xyxy(&center_x, &center_y, &width, &height);
                        bboxes.push(BoundingBox {
                            class_idx: best_class as i32,
                            score: best_score,
                            x1,
                            y1,
                            x2,
                            y2,
                            class_name: coco_class_name(best_class as i32),
                        });
                    }
                }
            }
        }

        let mut bboxes = nms(&bboxes, iou_thres);
        bboxes.truncate(max_detect);
        Ok(bboxes)
    }
}

// ## Detection-Related Utilities

#[cfg(feature = "execute")]
/// Load DynamicImage as Array4
/// Resulting normalized 4-dim array has shape [B, C, W, H] (batch size, channels, width, height)
/// ONNX detection model requires Array4-shaped, 0..1 normalized input
fn img_to_arr(img: &DynamicImage, width: u32, height: u32) -> Result<Array4<f32>, Error> {
    let (img_width, img_height) = img.dimensions();

    let buf_u8 = if (img_width == width) && (img_height == height) {
        img.to_rgb8().into_raw()
    } else {
        img.resize_exact(width, height, FilterType::Triangle)
            .into_rgb8()
            .into_raw()
    };

    // to float tensor
    let buf_f32: Vec<f32> = buf_u8.into_iter().map(|v| (v as f32) / 255.0).collect();

    // expand into 4dim array
    let arr4 = Array3::from_shape_vec((height as usize, width as usize, 3), buf_f32)?
        .permuted_axes([2, 0, 1])
        .insert_axis(Axis(0));
    Ok(arr4)
}

#[cfg(feature = "execute")]
fn img_to_arr_nchw_imagenet(
    img: &DynamicImage,
    width: u32,
    height: u32,
) -> Result<Array4<f32>, Error> {
    let rgb = img
        .resize_exact(width, height, FilterType::Triangle)
        .into_rgb8();
    let mean = [0.485, 0.456, 0.406];
    let std = [0.229, 0.224, 0.225];
    let mut input = Array4::<f32>::zeros((1, 3, height as usize, width as usize));
    for y in 0..height {
        for x in 0..width {
            let pixel = rgb.get_pixel(x, y);
            input[[0, 0, y as usize, x as usize]] = ((pixel[0] as f32 / 255.0) - mean[0]) / std[0];
            input[[0, 1, y as usize, x as usize]] = ((pixel[1] as f32 / 255.0) - mean[1]) / std[1];
            input[[0, 2, y as usize, x as usize]] = ((pixel[2] as f32 / 255.0) - mean[2]) / std[2];
        }
    }
    Ok(input)
}

#[cfg(feature = "execute")]
fn img_to_chw_bgr_detectron(img: &DynamicImage) -> Result<Array3<f32>, Error> {
    let ratio = detectron_resize_ratio(img);
    let width = (img.width() as f32 * ratio).round().max(1.0) as u32;
    let height = (img.height() as f32 * ratio).round().max(1.0) as u32;
    let rgb = img
        .resize_exact(width, height, FilterType::Triangle)
        .into_rgb8();
    let padded_w = width.div_ceil(32) * 32;
    let padded_h = height.div_ceil(32) * 32;
    let mean = [102.9801, 115.9465, 122.7717];
    let mut input = Array3::<f32>::zeros((3, padded_h as usize, padded_w as usize));
    for y in 0..height {
        for x in 0..width {
            let pixel = rgb.get_pixel(x, y);
            input[[0, y as usize, x as usize]] = pixel[2] as f32 - mean[0];
            input[[1, y as usize, x as usize]] = pixel[1] as f32 - mean[1];
            input[[2, y as usize, x as usize]] = pixel[0] as f32 - mean[2];
        }
    }
    Ok(input)
}

#[cfg(feature = "execute")]
fn detectron_resize_ratio(img: &DynamicImage) -> f32 {
    let min_side = img.width().min(img.height()).max(1) as f32;
    let max_side = img.width().max(img.height()).max(1) as f32;
    let mut ratio = 800.0 / min_side;
    if max_side * ratio > 1333.0 {
        ratio = 1333.0 / max_side;
    }
    ratio
}

#[cfg(feature = "execute")]
fn img_to_arr_nhwc_u8(img: &DynamicImage, width: u32, height: u32) -> Result<Array4<u8>, Error> {
    let rgb = if img.width() == width && img.height() == height {
        img.to_rgb8()
    } else {
        img.resize_exact(width, height, FilterType::Triangle)
            .into_rgb8()
    };
    let mut input = Array4::<u8>::zeros((1, height as usize, width as usize, 3));
    for y in 0..height {
        for x in 0..width {
            let pixel = rgb.get_pixel(x, y);
            input[[0, y as usize, x as usize, 0]] = pixel[0];
            input[[0, y as usize, x as usize, 1]] = pixel[1];
            input[[0, y as usize, x as usize, 2]] = pixel[2];
        }
    }
    Ok(input)
}

#[cfg(feature = "execute")]
fn img_to_arr_letterbox_nchw(
    img: &DynamicImage,
    width: u32,
    height: u32,
) -> Result<Array4<f32>, Error> {
    let rgb = letterbox_rgb(img, width, height);
    let mut input = Array4::<f32>::zeros((1, 3, height as usize, width as usize));
    for y in 0..height {
        for x in 0..width {
            let pixel = rgb.get_pixel(x, y);
            input[[0, 0, y as usize, x as usize]] = pixel[0] as f32 / 255.0;
            input[[0, 1, y as usize, x as usize]] = pixel[1] as f32 / 255.0;
            input[[0, 2, y as usize, x as usize]] = pixel[2] as f32 / 255.0;
        }
    }
    Ok(input)
}

#[cfg(feature = "execute")]
fn img_to_arr_letterbox_nhwc_f32(
    img: &DynamicImage,
    width: u32,
    height: u32,
) -> Result<Array4<f32>, Error> {
    let rgb = letterbox_rgb(img, width, height);
    let mut input = Array4::<f32>::zeros((1, height as usize, width as usize, 3));
    for y in 0..height {
        for x in 0..width {
            let pixel = rgb.get_pixel(x, y);
            input[[0, y as usize, x as usize, 0]] = pixel[0] as f32 / 255.0;
            input[[0, y as usize, x as usize, 1]] = pixel[1] as f32 / 255.0;
            input[[0, y as usize, x as usize, 2]] = pixel[2] as f32 / 255.0;
        }
    }
    Ok(input)
}

#[cfg(feature = "execute")]
fn letterbox_rgb(img: &DynamicImage, width: u32, height: u32) -> RgbImage {
    let scale = (width as f32 / img.width() as f32).min(height as f32 / img.height() as f32);
    let resized_w = (img.width() as f32 * scale).round().max(1.0) as u32;
    let resized_h = (img.height() as f32 * scale).round().max(1.0) as u32;
    let resized = img
        .resize_exact(resized_w, resized_h, FilterType::Triangle)
        .into_rgb8();
    let mut canvas = RgbImage::from_pixel(width, height, Rgb([128, 128, 128]));
    let dx = (width - resized_w) / 2;
    let dy = (height - resized_h) / 2;
    for y in 0..resized_h {
        for x in 0..resized_w {
            canvas.put_pixel(dx + x, dy + y, *resized.get_pixel(x, y));
        }
    }
    canvas
}

#[cfg(feature = "execute")]
fn extract_i32_vec(tensor: &DynValue) -> Result<Vec<i32>, Error> {
    if let Ok(values) = tensor.try_extract_array::<i64>() {
        return Ok(values.iter().map(|value| *value as i32).collect());
    }
    if let Ok(values) = tensor.try_extract_array::<i32>() {
        return Ok(values.iter().copied().collect());
    }
    if let Ok(values) = tensor.try_extract_array::<f32>() {
        return Ok(values.iter().map(|value| *value as i32).collect());
    }
    Err(anyhow!("Failed to extract integer tensor"))
}

#[cfg(feature = "execute")]
fn extract_i64_vec(tensor: &DynValue) -> Result<Vec<i64>, Error> {
    if let Ok(values) = tensor.try_extract_array::<i64>() {
        return Ok(values.iter().copied().collect());
    }
    if let Ok(values) = tensor.try_extract_array::<i32>() {
        return Ok(values.iter().map(|value| *value as i64).collect());
    }
    if let Ok(values) = tensor.try_extract_array::<f32>() {
        return Ok(values.iter().map(|value| *value as i64).collect());
    }
    Err(anyhow!("Failed to extract integer tensor"))
}

#[cfg(feature = "execute")]
fn extract_f32_vec(tensor: &DynValue) -> Result<Vec<f32>, Error> {
    if let Ok(values) = tensor.try_extract_array::<f32>() {
        return Ok(values.iter().copied().collect());
    }
    if let Ok(values) = tensor.try_extract_array::<i64>() {
        return Ok(values.iter().map(|value| *value as f32).collect());
    }
    if let Ok(values) = tensor.try_extract_array::<i32>() {
        return Ok(values.iter().map(|value| *value as f32).collect());
    }
    Err(anyhow!("Failed to extract numeric tensor"))
}

#[cfg(feature = "execute")]
/// Convert center-x, center-y, width, height to left, top, right, bottom representation
fn xywh_to_xyxy(x: &f32, y: &f32, w: &f32, h: &f32) -> (f32, f32, f32, f32) {
    let x1 = x - w / 2.0;
    let y1 = y - h / 2.0;
    let x2 = x + w / 2.0;
    let y2 = y + h / 2.0;
    (x1, y1, x2, y2)
}

#[cfg(feature = "execute")]
fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value).exp())
}

#[cfg(feature = "execute")]
fn yolo_grid_value(
    data: &[f32],
    channel: usize,
    y: usize,
    x: usize,
    grid_h: usize,
    grid_w: usize,
) -> f32 {
    data[(channel * grid_h + y) * grid_w + x]
}

#[cfg(feature = "execute")]
fn yolo_v3_selected_boxes(
    boxes: &[f32],
    boxes_batches: usize,
    boxes_per_batch: usize,
    scores: &[f32],
    scores_batches: usize,
    classes_per_batch: usize,
    score_boxes: usize,
    indices: &[i64],
    conf_thres: f32,
    max_detect: usize,
) -> Result<Vec<BoundingBox>, Error> {
    let expected_boxes = boxes_batches
        .checked_mul(boxes_per_batch)
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| anyhow!("YOLOv3 boxes shape is too large"))?;
    if boxes.len() < expected_boxes {
        return Err(anyhow!("YOLOv3 boxes output is smaller than its shape"));
    }

    let expected_scores = scores_batches
        .checked_mul(classes_per_batch)
        .and_then(|value| value.checked_mul(score_boxes))
        .ok_or_else(|| anyhow!("YOLOv3 scores shape is too large"))?;
    if scores.len() < expected_scores {
        return Err(anyhow!("YOLOv3 scores output is smaller than its shape"));
    }

    let mut bboxes = Vec::new();
    for idx in indices.chunks_exact(3) {
        if idx.iter().any(|value| *value < 0) {
            continue;
        }

        let batch = idx[0] as usize;
        let class_idx = idx[1] as usize;
        let box_idx = idx[2] as usize;
        if batch >= boxes_batches
            || batch >= scores_batches
            || class_idx >= classes_per_batch
            || box_idx >= boxes_per_batch
            || box_idx >= score_boxes
        {
            continue;
        }

        let score_idx = batch * classes_per_batch * score_boxes + class_idx * score_boxes + box_idx;
        let score = scores[score_idx];
        if !score.is_finite() || score <= conf_thres {
            continue;
        }

        let box_base = (batch * boxes_per_batch + box_idx) * 4;
        let y1 = boxes[box_base];
        let x1 = boxes[box_base + 1];
        let y2 = boxes[box_base + 2];
        let x2 = boxes[box_base + 3];
        if !x1.is_finite() || !y1.is_finite() || !x2.is_finite() || !y2.is_finite() {
            continue;
        }

        bboxes.push(BoundingBox {
            class_idx: class_idx as i32,
            score,
            x1,
            y1,
            x2,
            y2,
            class_name: coco_class_name(class_idx as i32),
        });
    }

    bboxes.sort_unstable_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    bboxes.truncate(max_detect);
    Ok(bboxes)
}

#[cfg(feature = "execute")]
fn nchw_value(data: &[f32], channel: usize, y: usize, x: usize, h: usize, w: usize) -> f32 {
    data[(channel * h + y) * w + x]
}

#[cfg(feature = "execute")]
fn yolo_v2_anchors(num_classes: usize) -> [(f32, f32); 5] {
    if num_classes == 20 {
        [
            (1.08, 1.19),
            (3.42, 4.41),
            (6.63, 11.38),
            (9.42, 5.11),
            (16.62, 10.52),
        ]
    } else {
        [
            (0.57273, 0.677385),
            (1.87446, 2.06253),
            (3.33843, 5.47434),
            (7.88282, 3.52778),
            (9.77052, 9.16828),
        ]
    }
}

#[cfg(feature = "execute")]
fn yolo_v4_anchors(grid_w: usize) -> [(f32, f32); 3] {
    match grid_w {
        52 => [(12.0, 16.0), (19.0, 36.0), (40.0, 28.0)],
        26 => [(36.0, 75.0), (76.0, 55.0), (72.0, 146.0)],
        _ => [(142.0, 110.0), (192.0, 243.0), (459.0, 401.0)],
    }
}

#[cfg(feature = "execute")]
fn yolo_v4_xyscale(grid_w: usize) -> f32 {
    match grid_w {
        52 => 1.2,
        26 => 1.1,
        _ => 1.05,
    }
}

#[cfg(feature = "execute")]
fn retinanet_anchors(stride: f32) -> [(f32, f32); 9] {
    let ratios = [1.0_f32, 2.0, 0.5];
    let scales = [
        4.0_f32,
        4.0 * 2.0_f32.powf(1.0 / 3.0),
        4.0 * 2.0_f32.powf(2.0 / 3.0),
    ];
    let mut anchors = [(0.0, 0.0); 9];
    let mut idx = 0;
    for ratio in ratios {
        for scale in scales {
            let area = (stride * scale).powi(2);
            let width = (area / ratio).sqrt();
            let height = width * ratio;
            anchors[idx] = (width, height);
            idx += 1;
        }
    }
    anchors
}

#[cfg(feature = "execute")]
fn unletterbox_boxes(
    bboxes: &mut [BoundingBox],
    original_width: u32,
    original_height: u32,
    input_width: u32,
    input_height: u32,
) {
    let ratio = (input_width as f32 / original_width as f32)
        .min(input_height as f32 / original_height as f32);
    let dw = (input_width as f32 - original_width as f32 * ratio) / 2.0;
    let dh = (input_height as f32 - original_height as f32 * ratio) / 2.0;
    for bbox in bboxes {
        bbox.x1 = ((bbox.x1 - dw) / ratio).clamp(0.0, original_width.saturating_sub(1) as f32);
        bbox.x2 = ((bbox.x2 - dw) / ratio).clamp(0.0, original_width.saturating_sub(1) as f32);
        bbox.y1 = ((bbox.y1 - dh) / ratio).clamp(0.0, original_height.saturating_sub(1) as f32);
        bbox.y2 = ((bbox.y2 - dh) / ratio).clamp(0.0, original_height.saturating_sub(1) as f32);
    }
}

#[cfg(feature = "execute")]
const COCO_80_CLASS_NAMES: [&str; 80] = [
    "person",
    "bicycle",
    "car",
    "motorcycle",
    "airplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "backpack",
    "umbrella",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "couch",
    "potted plant",
    "bed",
    "dining table",
    "toilet",
    "tv",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
];

#[cfg(feature = "execute")]
const VOC_20_CLASS_NAMES: [&str; 20] = [
    "aeroplane",
    "bicycle",
    "bird",
    "boat",
    "bottle",
    "bus",
    "car",
    "cat",
    "chair",
    "cow",
    "diningtable",
    "dog",
    "horse",
    "motorbike",
    "person",
    "pottedplant",
    "sheep",
    "sofa",
    "train",
    "tvmonitor",
];

#[cfg(feature = "execute")]
fn class_name_from_labels(class_idx: i32, labels: &[&str]) -> Option<String> {
    if class_idx < 0 {
        return None;
    }

    labels.get(class_idx as usize).map(|name| (*name).into())
}

#[cfg(feature = "execute")]
fn coco_class_name(class_idx: i32) -> Option<String> {
    class_name_from_labels(class_idx, &COCO_80_CLASS_NAMES)
}

#[cfg(feature = "execute")]
fn coco_class_name_with_background(class_idx: i32) -> Option<String> {
    if class_idx <= 0 {
        return None;
    }

    coco_class_name(class_idx - 1)
}

#[cfg(feature = "execute")]
fn voc_class_name(class_idx: i32) -> Option<String> {
    class_name_from_labels(class_idx, &VOC_20_CLASS_NAMES)
}

#[cfg(feature = "execute")]
fn class_name_for_contiguous_label(class_idx: i32, num_classes: usize) -> Option<String> {
    match num_classes {
        20 => voc_class_name(class_idx),
        80 => coco_class_name(class_idx),
        _ => None,
    }
}

#[cfg(feature = "execute")]
fn coco_category_id_class_name(class_idx: i32) -> Option<String> {
    let zero_based_idx = match class_idx {
        1 => 0,
        2 => 1,
        3 => 2,
        4 => 3,
        5 => 4,
        6 => 5,
        7 => 6,
        8 => 7,
        9 => 8,
        10 => 9,
        11 => 10,
        13 => 11,
        14 => 12,
        15 => 13,
        16 => 14,
        17 => 15,
        18 => 16,
        19 => 17,
        20 => 18,
        21 => 19,
        22 => 20,
        23 => 21,
        24 => 22,
        25 => 23,
        27 => 24,
        28 => 25,
        31 => 26,
        32 => 27,
        33 => 28,
        34 => 29,
        35 => 30,
        36 => 31,
        37 => 32,
        38 => 33,
        39 => 34,
        40 => 35,
        41 => 36,
        42 => 37,
        43 => 38,
        44 => 39,
        46 => 40,
        47 => 41,
        48 => 42,
        49 => 43,
        50 => 44,
        51 => 45,
        52 => 46,
        53 => 47,
        54 => 48,
        55 => 49,
        56 => 50,
        57 => 51,
        58 => 52,
        59 => 53,
        60 => 54,
        61 => 55,
        62 => 56,
        63 => 57,
        64 => 58,
        65 => 59,
        67 => 60,
        70 => 61,
        72 => 62,
        73 => 63,
        74 => 64,
        75 => 65,
        76 => 66,
        77 => 67,
        78 => 68,
        79 => 69,
        80 => 70,
        81 => 71,
        82 => 72,
        84 => 73,
        85 => 74,
        86 => 75,
        87 => 76,
        88 => 77,
        89 => 78,
        90 => 79,
        _ => return None,
    };

    coco_class_name(zero_based_idx)
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;

    #[test]
    fn detection_class_name_maps_known_label_spaces() {
        assert_eq!(coco_class_name(0).as_deref(), Some("person"));
        assert_eq!(coco_class_name(16).as_deref(), Some("dog"));
        assert_eq!(coco_class_name(80).as_deref(), None);

        assert_eq!(
            coco_class_name_with_background(1).as_deref(),
            Some("person")
        );
        assert_eq!(
            coco_class_name_with_background(18).as_deref(),
            Some("horse")
        );
        assert_eq!(coco_class_name_with_background(0), None);

        assert_eq!(voc_class_name(14).as_deref(), Some("person"));
        assert_eq!(voc_class_name(20).as_deref(), None);

        assert_eq!(
            class_name_for_contiguous_label(0, 80).as_deref(),
            Some("person")
        );
        assert_eq!(
            class_name_for_contiguous_label(14, 20).as_deref(),
            Some("person")
        );
        assert_eq!(class_name_for_contiguous_label(0, 3), None);

        assert_eq!(coco_category_id_class_name(1).as_deref(), Some("person"));
        assert_eq!(coco_category_id_class_name(18).as_deref(), Some("dog"));
        assert_eq!(coco_category_id_class_name(12), None);
    }

    #[test]
    fn yolo_v3_selected_boxes_convert_nms_yxyx_to_xyxy() {
        let boxes = vec![10.0, 20.0, 30.0, 40.0];
        let mut scores = vec![0.0; 80];
        scores[0] = 0.9;
        let indices = vec![0, 0, 0];

        let bboxes =
            yolo_v3_selected_boxes(&boxes, 1, 1, &scores, 1, 80, 1, &indices, 0.5, 10).unwrap();

        assert_eq!(bboxes.len(), 1);
        assert_eq!(bboxes[0].x1, 20.0);
        assert_eq!(bboxes[0].y1, 10.0);
        assert_eq!(bboxes[0].x2, 40.0);
        assert_eq!(bboxes[0].y2, 30.0);
        assert_eq!(bboxes[0].class_name.as_deref(), Some("person"));
    }

    #[test]
    fn detectron_preprocessing_resizes_and_zero_pads_after_normalization() {
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(801, 800, Rgb([10, 20, 30])));

        let input = img_to_chw_bgr_detectron(&img).unwrap();

        assert_eq!(detectron_resize_ratio(&img), 1.0);
        assert_eq!(input.shape(), &[3, 800, 832]);
        assert!((input[[0, 0, 0]] - (30.0 - 102.9801)).abs() < 0.0001);
        assert!((input[[1, 0, 0]] - (20.0 - 115.9465)).abs() < 0.0001);
        assert!((input[[2, 0, 0]] - (10.0 - 122.7717)).abs() < 0.0001);
        assert_eq!(input[[0, 0, 801]], 0.0);
        assert_eq!(input[[1, 0, 801]], 0.0);
        assert_eq!(input[[2, 0, 801]], 0.0);
    }

    #[test]
    fn detectron_resize_ratio_caps_long_side() {
        let img = DynamicImage::ImageRgb8(RgbImage::new(4000, 1000));

        assert!((detectron_resize_ratio(&img) - (1333.0 / 4000.0)).abs() < 0.0001);
    }

    #[test]
    fn retinanet_dynamic_input_uses_original_image_size() {
        let provider = RetinaNetLike {
            input_name: "input".into(),
            output_names: Vec::new(),
            input_width: 0,
            input_height: 0,
            resize_input: false,
        };
        let img = DynamicImage::ImageRgb8(RgbImage::new(641, 479));

        assert_eq!(provider.input_size_for_image(&img), (641, 479));
    }
}

#[cfg(feature = "execute")]
fn bounding_box_from_array(arr: ArrayView1<f32>) -> BoundingBox {
    let bbox_xywh = arr.slice(s![..4]).to_vec();
    let confs = arr.slice(s![4..]).to_vec();
    let (class_idx, conf) = confs
        .iter()
        .enumerate()
        .filter_map(
            |(idx, &num)| {
                if num.is_nan() { None } else { Some((idx, num)) }
            },
        )
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
        .unwrap();
    let (x1, y1, x2, y2) = xywh_to_xyxy(&bbox_xywh[0], &bbox_xywh[1], &bbox_xywh[2], &bbox_xywh[3]);
    BoundingBox {
        x1,
        y1,
        x2,
        y2,
        score: conf,
        class_idx: class_idx as i32,
        class_name: class_name_for_contiguous_label(class_idx as i32, confs.len()),
    }
}

#[cfg(feature = "execute")]
/// Class-Sensitive Non Maxima Suppression for Overlapping Bounding Boxes
/// Iteratively removes lower scoring bboxes which have an IoU above iou_thresold.
/// Inspired by: https://pytorch.org/vision/master/_modules/torchvision/ops/boxes.html#nms
fn nms(boxes: &[BoundingBox], iou_threshold: f32) -> Vec<BoundingBox> {
    if boxes.is_empty() {
        return Vec::new();
    }

    // Compute the maximum coordinate value among all boxes
    let max_coordinate = boxes.iter().fold(0.0_f32, |max_coord, bbox| {
        max_coord.max(bbox.x2).max(bbox.y2)
    });
    let offset = max_coordinate + 1.0;

    // Create a vector of shifted boxes with their original indices
    let mut boxes_shifted: Vec<(BoundingBox, usize)> = boxes
        .iter()
        .enumerate()
        .map(|(i, bbox)| {
            let class_offset = offset * bbox.class_idx as f32;
            let shifted_bbox = BoundingBox {
                x1: bbox.x1 + class_offset,
                y1: bbox.y1 + class_offset,
                x2: bbox.x2 + class_offset,
                y2: bbox.y2 + class_offset,
                score: bbox.score,
                class_idx: bbox.class_idx, // Keep class_idx the same
                class_name: bbox.class_name.clone(),
            };
            (shifted_bbox, i) // Keep track of the original index
        })
        .collect();

    // Sort boxes in decreasing order based on scores
    boxes_shifted
        .sort_unstable_by(|a, b| b.0.score.partial_cmp(&a.0.score).unwrap_or(Ordering::Equal));

    let mut keep_indices = Vec::new();

    while let Some((current_box, original_index)) = boxes_shifted.first().cloned() {
        keep_indices.push(original_index);
        boxes_shifted.remove(0);

        // Retain boxes that have an IoU less than or equal to the threshold with the current box
        boxes_shifted.retain(|(bbox, _)| current_box.iou(bbox) <= iou_threshold);
    }

    // Collect the kept boxes from the original input
    let mut kept_boxes: Vec<BoundingBox> = keep_indices
        .into_iter()
        .map(|idx| boxes[idx].clone())
        .collect();

    // Sort the kept boxes in decreasing order of their scores
    kept_boxes.sort_unstable_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));

    kept_boxes
}

#[crate::register_node]
#[derive(Default)]
/// # Object Detection Node
/// Evaluate ONNX-based Object Detection Models for Images
pub struct ObjectDetectionNode {}

impl ObjectDetectionNode {
    /// Create new LoadOnnxNode Instance
    pub fn new() -> Self {
        ObjectDetectionNode {}
    }
}

#[async_trait]
impl NodeLogic for ObjectDetectionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "object_detection",
            "Object Detection",
            "Object Detection in Images with ONNX-Models. Download models from: TinyYOLOv2 (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation/tiny-yolov2), YOLO (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation), SSD-MobileNet (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation/ssd-mobilenetv1)",
            "AI/ML/ONNX",
        );
        node.set_version(1);

        node.add_icon("/flow/icons/find_model.svg");

        // inputs
        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("model", "Model", "ONNX Model Session", VariableType::Struct)
            .set_schema::<NodeOnnxSession>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("image_in", "Image", "Image Object", VariableType::Struct)
            .set_schema::<NodeImage>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("conf", "Conf", "Confidence Threshold", VariableType::Float)
            .set_options(PinOptions::new().set_range((0., 1.)).build())
            .set_default_value(Some(json!(0.25)));

        node.add_input_pin(
            "iou",
            "IoU",
            "Intersection Over Union Threshold for NMS",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0., 1.)).build())
        .set_default_value(Some(json!(0.7)));

        node.add_input_pin(
            "max",
            "Max",
            "Maximum Number of Detections",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((0., 1000.)).build())
        .set_default_value(Some(json!(300)));

        // outputs
        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "bboxes",
            "Boxes",
            "Bounding Box Predictions",
            VariableType::Struct,
        )
        .set_schema::<BoundingBox>()
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            // fetch params
            let node_session: NodeOnnxSession = context.evaluate_pin("model").await?;
            let node_img: NodeImage = context.evaluate_pin("image_in").await?;
            let conf_thres: f32 = context.evaluate_pin("conf").await?;
            let iou_thres: f32 = context.evaluate_pin("iou").await?;
            let max_detect: usize = context.evaluate_pin("max").await?;

            // run inference
            let predictions = {
                let img = node_img.get_image(context).await?;
                let img_guard = img.lock().await;
                let session = node_session.get_session(context).await?;
                let mut session_guard = session.lock().await;
                if matches!(session_guard.provider, Provider::Generic) {
                    let provider = super::load::determine_provider(&session_guard.session)?;
                    if !matches!(provider, Provider::Generic) {
                        session_guard.provider = provider;
                    }
                }

                // Copy provider params to avoid overlapping borrows
                match &session_guard.provider {
                    Provider::DfineLike(m) => {
                        let prov = super::detection::DfineLike {
                            input_width: m.input_width,
                            input_height: m.input_height,
                        };
                        prov.run(
                            &mut session_guard.session,
                            &img_guard,
                            conf_thres,
                            iou_thres,
                            max_detect,
                        )
                    }
                    Provider::YoloLike(m) => {
                        let prov = super::detection::YoloLike {
                            input_width: m.input_width,
                            input_height: m.input_height,
                        };
                        prov.run(
                            &mut session_guard.session,
                            &img_guard,
                            conf_thres,
                            iou_thres,
                            max_detect,
                        )
                    }
                    Provider::BoxLabelsScoresLike(m) => {
                        let prov = m.clone();
                        prov.run(
                            &mut session_guard.session,
                            &img_guard,
                            conf_thres,
                            iou_thres,
                            max_detect,
                        )
                    }
                    Provider::SsdMobileNetLike(m) => {
                        let prov = m.clone();
                        prov.run(
                            &mut session_guard.session,
                            &img_guard,
                            conf_thres,
                            iou_thres,
                            max_detect,
                        )
                    }
                    Provider::YoloV2GridLike(m) => {
                        let prov = m.clone();
                        prov.run(
                            &mut session_guard.session,
                            &img_guard,
                            conf_thres,
                            iou_thres,
                            max_detect,
                        )
                    }
                    Provider::YoloV3Like(m) => {
                        let prov = m.clone();
                        prov.run(
                            &mut session_guard.session,
                            &img_guard,
                            conf_thres,
                            iou_thres,
                            max_detect,
                        )
                    }
                    Provider::YoloV4Like(m) => {
                        let prov = m.clone();
                        prov.run(
                            &mut session_guard.session,
                            &img_guard,
                            conf_thres,
                            iou_thres,
                            max_detect,
                        )
                    }
                    Provider::RetinaNetLike(m) => {
                        let prov = m.clone();
                        prov.run(
                            &mut session_guard.session,
                            &img_guard,
                            conf_thres,
                            iou_thres,
                            max_detect,
                        )
                    }
                    provider => Err(incompatible_object_detection_model_error(
                        provider,
                        &session_guard.session,
                    )),
                }?
            };

            // set outputs
            context.set_pin_value("bboxes", json!(predictions)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        {
            Err(anyhow!(
                "ONNX execution requires the 'execute' feature. Rebuild with --features execute"
            ))
        }
    }
}

#[cfg(feature = "execute")]
fn incompatible_object_detection_model_error(provider: &Provider, session: &Session) -> Error {
    let inputs = session
        .inputs()
        .iter()
        .map(|input| format!("{}:{:?}", input.name(), input.dtype()))
        .collect::<Vec<_>>()
        .join(", ");
    let outputs = session
        .outputs()
        .iter()
        .map(|output| format!("{}:{:?}", output.name(), output.dtype()))
        .collect::<Vec<_>>()
        .join(", ");

    anyhow!(
        "Incompatible ONNX model for Object Detection: detected provider {}; inputs [{}]; outputs [{}]",
        provider_name(provider),
        inputs,
        outputs
    )
}

#[cfg(feature = "execute")]
fn provider_name(provider: &Provider) -> &'static str {
    match provider {
        Provider::DfineLike(_) => "DfineLike",
        Provider::YoloLike(_) => "YoloLike",
        Provider::BoxLabelsScoresLike(_) => "BoxLabelsScoresLike",
        Provider::SsdMobileNetLike(_) => "SsdMobileNetLike",
        Provider::YoloV2GridLike(_) => "YoloV2GridLike",
        Provider::YoloV3Like(_) => "YoloV3Like",
        Provider::YoloV4Like(_) => "YoloV4Like",
        Provider::RetinaNetLike(_) => "RetinaNetLike",
        Provider::TimmLike(_) => "TimmLike",
        Provider::Generic => "Generic",
    }
}

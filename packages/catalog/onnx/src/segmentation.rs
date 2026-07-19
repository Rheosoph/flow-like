/// # ONNX Semantic/Instance Segmentation Nodes
use crate::onnx::NodeOnnxSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::{NodeImage, SegmentationMask};
#[cfg(feature = "execute")]
use flow_like_model_provider::ml::{
    ndarray::{Array3, Array4, Axis, s},
    ort::{
        inputs,
        session::{Session, SessionInputValue, SessionOutputs},
        value::Value,
    },
};
#[cfg(feature = "execute")]
use flow_like_types::{
    Error,
    image::{DynamicImage, GenericImageView, imageops::FilterType},
};
use flow_like_types::{Result, anyhow, async_trait, json::json};
#[cfg(feature = "execute")]
use std::borrow::Cow;

#[cfg(feature = "execute")]
/// Segmentation Trait for Common Behavior
pub trait Segmentation {
    /// Preprocess image for segmentation model
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error>;

    /// Postprocess model outputs to segmentation mask
    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        original_width: u32,
        original_height: u32,
    ) -> Result<SegmentationMask, Error>;

    /// End-to-End Inference
    fn run(&self, session: &mut Session, img: &DynamicImage) -> Result<SegmentationMask, Error>;
}

/// UNet-like segmentation model provider
/// Supports models with standard input/output naming
pub struct UNetLike {
    pub input_width: u32,
    pub input_height: u32,
    pub num_classes: u32,
    pub input_name: String,
    pub output_name: String,
}

impl Default for UNetLike {
    fn default() -> Self {
        Self {
            input_width: 512,
            input_height: 512,
            num_classes: 21, // VOC default
            input_name: "input".to_string(),
            output_name: "output".to_string(),
        }
    }
}

#[cfg(feature = "execute")]
impl Segmentation for UNetLike {
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error> {
        let arr = img_to_arr(img, self.input_width, self.input_height)?;
        let value = Value::from_array(arr)?;
        let session_inputs = inputs![
            self.input_name.as_str() => value
        ];
        Ok(session_inputs)
    }

    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        original_width: u32,
        original_height: u32,
    ) -> Result<SegmentationMask, Error> {
        // Output shape: [1, num_classes, H, W] or [1, H, W]
        let output = outputs[self.output_name.as_str()].try_extract_array::<f32>()?;
        let shape = output.shape();

        let (mask_height, mask_width, data) = if shape.len() == 4 {
            // [B, C, H, W] - argmax over classes
            let h = shape[2] as u32;
            let w = shape[3] as u32;
            let mut data = Vec::with_capacity((h * w) as usize);

            for y in 0..h as usize {
                for x in 0..w as usize {
                    let mut max_class = 0u8;
                    let mut max_score = f32::NEG_INFINITY;
                    for c in 0..shape[1] {
                        let score = output[[0, c, y, x]];
                        if score > max_score {
                            max_score = score;
                            max_class = c as u8;
                        }
                    }
                    data.push(max_class);
                }
            }
            (h, w, data)
        } else if shape.len() == 3 {
            // [B, H, W] - already class indices
            let h = shape[1] as u32;
            let w = shape[2] as u32;
            let data: Vec<u8> = output
                .slice(s![0, .., ..])
                .iter()
                .map(|&v| v as u8)
                .collect();
            (h, w, data)
        } else {
            return Err(anyhow!("Unexpected output shape: {:?}", shape));
        };

        // Resize mask to original image size if needed
        let (final_width, final_height, final_data) =
            if mask_width != original_width || mask_height != original_height {
                resize_mask(
                    &data,
                    mask_width,
                    mask_height,
                    original_width,
                    original_height,
                )
            } else {
                (mask_width, mask_height, data)
            };

        Ok(SegmentationMask {
            width: final_width,
            height: final_height,
            data: final_data,
            num_classes: self.num_classes,
            class_labels: None,
        })
    }

    fn run(&self, session: &mut Session, img: &DynamicImage) -> Result<SegmentationMask, Error> {
        let (original_width, original_height) = img.dimensions();
        let inputs = self.make_inputs(img)?;
        let outputs = session.run(inputs)?;
        self.make_results(outputs, original_width, original_height)
    }
}

/// DeepLab-like segmentation model provider
pub struct DeepLabLike {
    pub input_width: u32,
    pub input_height: u32,
    pub num_classes: u32,
}

impl Default for DeepLabLike {
    fn default() -> Self {
        Self {
            input_width: 513,
            input_height: 513,
            num_classes: 21,
        }
    }
}

#[cfg(feature = "execute")]
impl Segmentation for DeepLabLike {
    fn make_inputs(
        &self,
        img: &DynamicImage,
    ) -> Result<Vec<(Cow<'_, str>, SessionInputValue<'_>)>, Error> {
        let arr = img_to_arr_normalized(
            img,
            self.input_width,
            self.input_height,
            &[0.485, 0.456, 0.406],
            &[0.229, 0.224, 0.225],
        )?;
        let value = Value::from_array(arr)?;
        let session_inputs = inputs![
            "input" => value
        ];
        Ok(session_inputs)
    }

    fn make_results(
        &self,
        outputs: SessionOutputs<'_>,
        original_width: u32,
        original_height: u32,
    ) -> Result<SegmentationMask, Error> {
        // DeepLab outputs: [B, H, W] with class indices
        let output = outputs["output"].try_extract_array::<i64>()?;
        let shape = output.shape();
        let h = shape[1] as u32;
        let w = shape[2] as u32;

        let data: Vec<u8> = output
            .slice(s![0, .., ..])
            .iter()
            .map(|&v| v as u8)
            .collect();

        let (final_width, final_height, final_data) = if w != original_width || h != original_height
        {
            resize_mask(&data, w, h, original_width, original_height)
        } else {
            (w, h, data)
        };

        Ok(SegmentationMask {
            width: final_width,
            height: final_height,
            data: final_data,
            num_classes: self.num_classes,
            class_labels: None,
        })
    }

    fn run(&self, session: &mut Session, img: &DynamicImage) -> Result<SegmentationMask, Error> {
        let (original_width, original_height) = img.dimensions();
        let inputs = self.make_inputs(img)?;
        let outputs = session.run(inputs)?;
        self.make_results(outputs, original_width, original_height)
    }
}

// ## Segmentation Utilities

#[cfg(feature = "execute")]
/// Load DynamicImage as Array4 normalized to 0-1
fn img_to_arr(img: &DynamicImage, width: u32, height: u32) -> Result<Array4<f32>, Error> {
    let (img_width, img_height) = img.dimensions();
    let buf_u8 = if (img_width == width) && (img_height == height) {
        img.to_rgb8().into_raw()
    } else {
        img.resize_exact(width, height, FilterType::Triangle)
            .into_rgb8()
            .into_raw()
    };

    let buf_f32: Vec<f32> = buf_u8.into_iter().map(|v| (v as f32) / 255.0).collect();
    let arr4 = Array3::from_shape_vec((height as usize, width as usize, 3), buf_f32)?
        .permuted_axes([2, 0, 1])
        .insert_axis(Axis(0));
    Ok(arr4)
}

#[cfg(feature = "execute")]
/// Load DynamicImage as normalized Array4 with mean/std
fn img_to_arr_normalized(
    img: &DynamicImage,
    width: u32,
    height: u32,
    mean: &[f32; 3],
    std: &[f32; 3],
) -> Result<Array4<f32>, Error> {
    let (img_width, img_height) = img.dimensions();
    let buf_u8 = if (img_width == width) && (img_height == height) {
        img.to_rgb8().into_raw()
    } else {
        img.resize_exact(width, height, FilterType::Triangle)
            .into_rgb8()
            .into_raw()
    };

    let buf_f32: Vec<f32> = buf_u8.iter().map(|&v| (v as f32) / 255.0).collect();
    let mut arr3 = Array3::from_shape_vec((height as usize, width as usize, 3), buf_f32)?;

    for c in 0..3 {
        arr3.slice_mut(s![.., .., c]).map_inplace(|x| {
            *x = (*x - mean[c]) / std[c];
        });
    }

    let arr4 = arr3.permuted_axes([2, 0, 1]).insert_axis(Axis(0));
    Ok(arr4)
}

#[cfg(feature = "execute")]
/// Resize segmentation mask using nearest neighbor interpolation
fn resize_mask(data: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> (u32, u32, Vec<u8>) {
    let mut result = Vec::with_capacity((dst_w * dst_h) as usize);

    for dst_y in 0..dst_h {
        for dst_x in 0..dst_w {
            let src_x = (dst_x as f32 * src_w as f32 / dst_w as f32) as u32;
            let src_y = (dst_y as f32 * src_h as f32 / dst_h as f32) as u32;
            let src_x = src_x.min(src_w - 1);
            let src_y = src_y.min(src_h - 1);
            let idx = (src_y * src_w + src_x) as usize;
            result.push(data.get(idx).copied().unwrap_or(0));
        }
    }

    (dst_w, dst_h, result)
}

#[crate::register_node]
#[derive(Default)]
/// # Semantic Segmentation Node
/// Perform semantic segmentation using ONNX models
pub struct SemanticSegmentationNode {}

impl SemanticSegmentationNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SemanticSegmentationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "semantic_segmentation",
            "Semantic Segmentation",
            "Segment images into semantic classes using ONNX models. Download models from: DeepLabV3 (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation/duc), FCN (https://github.com/onnx/models/tree/main/validated/vision/object_detection_segmentation/fcn)",
            "AI/ML/ONNX",
        );
        node.set_version(1);

        node.add_icon("/flow/icons/find_model.svg");

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

        node.add_input_pin(
            "num_classes",
            "Classes",
            "Number of segmentation classes",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(21)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "mask",
            "Mask",
            "Segmentation mask output",
            VariableType::Struct,
        )
        .set_schema::<SegmentationMask>();

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let node_session: NodeOnnxSession = context.evaluate_pin("model").await?;
            let node_img: NodeImage = context.evaluate_pin("image_in").await?;
            let num_classes: i64 = context.evaluate_pin("num_classes").await.unwrap_or(21);

            let mask = {
                let img = node_img.get_image(context).await?;
                let img_guard = img.lock().await;
                let session = node_session.get_session(context).await?;
                let mut session_guard = session.lock().await;

                // Determine input shape from session
                let (input_width, input_height) =
                    if let Some(input) = session_guard.session.inputs().first() {
                        if let Some(dims) = input.dtype().tensor_shape() {
                            let d = dims.len();
                            if d >= 2 {
                                (dims[d - 1] as u32, dims[d - 2] as u32)
                            } else {
                                (512, 512)
                            }
                        } else {
                            (512, 512)
                        }
                    } else {
                        (512, 512)
                    };

                // Determine input/output names
                let input_name = session_guard
                    .session
                    .inputs()
                    .first()
                    .map(|i| i.name().to_string())
                    .unwrap_or_else(|| "input".to_string());
                let output_name = session_guard
                    .session
                    .outputs()
                    .first()
                    .map(|o| o.name().to_string())
                    .unwrap_or_else(|| "output".to_string());

                let provider = UNetLike {
                    input_width,
                    input_height,
                    num_classes: num_classes as u32,
                    input_name,
                    output_name,
                };

                provider.run(&mut session_guard.session, &img_guard)?
            };

            context.set_pin_value("mask", json!(mask)).await?;
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

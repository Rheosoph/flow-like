use flow_like::flow::{execution::context::ExecutionContext, node::NodeLogic};
use flow_like_catalog_core::NodeImage;
use flow_like_ml_core::{PreprocessingStep, Sample, TensorData};
use flow_like_types::{Result, anyhow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ImagePreprocessRequest {
    pub image: NodeImage,
    pub width: u32,
    pub height: u32,
    pub mean: [f32; 3],
    pub standard_deviation: [f32; 3],
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreprocessedImage {
    pub tensor: TensorData,
    pub preprocessing: Vec<PreprocessingStep>,
}
#[cfg(any(feature = "execute", test))]
fn preprocess(
    image: flow_like_types::image::DynamicImage,
    width: u32,
    height: u32,
    mean: [f32; 3],
    std: [f32; 3],
) -> Result<PreprocessedImage> {
    if width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > 5_592_405
        || !mean.iter().all(|v| v.is_finite())
        || !std.iter().all(|v| v.is_finite() && *v > 0.0)
    {
        return Err(anyhow!(
            "Image size or normalization parameters are invalid"
        ));
    }
    let image = image
        .resize_exact(
            width,
            height,
            flow_like_types::image::imageops::FilterType::Triangle,
        )
        .to_rgb8();
    let pixels = (width as usize) * (height as usize);
    let mut values = vec![0.0; pixels * 3];
    for (i, pixel) in image.pixels().enumerate() {
        for channel in 0..3 {
            values[channel * pixels + i] =
                (pixel[channel] as f32 / 255.0 - mean[channel]) / std[channel];
        }
    }
    Ok(PreprocessedImage {
        tensor: TensorData {
            shape: vec![3, height as usize, width as usize],
            values,
        },
        preprocessing: vec![
            PreprocessingStep::Resize {
                width: width as usize,
                height: height as usize,
            },
            PreprocessingStep::Normalize {
                mean: mean.to_vec(),
                std: std.to_vec(),
            },
        ],
    })
}
#[crate::register_node]
#[derive(Default)]
pub struct PreprocessInspectionImageNode;
#[flow_like_types::async_trait]
impl NodeLogic for PreprocessInspectionImageNode {
    fn get_node(&self) -> flow_like::flow::node::Node {
        super::operation_node::<ImagePreprocessRequest, PreprocessedImage>(
            "ml_preprocess_inspection_image",
            "Preprocess Inspection Image",
            "Resize an image and produce an RGB channel-first tensor with explicit normalization",
            "preprocessImage",
            "AI/ML/Vision",
        )
    }
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let input: ImagePreprocessRequest = context.evaluate_pin("request").await?;
            let image = input.image.get_image(context).await?.lock().await.clone();
            let output = tokio::task::spawn_blocking(move || {
                preprocess(
                    image,
                    input.width,
                    input.height,
                    input.mean,
                    input.standard_deviation,
                )
            })
            .await??;
            context
                .set_pin_value("result", flow_like_types::json::json!(output))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }
        #[cfg(not(feature = "execute"))]
        {
            let _ = context;
            Err(anyhow!("Image preprocessing requires execution support"))
        }
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct FlipSampleRequest {
    pub sample: Sample,
    pub labels: Vec<String>,
}
fn flip(mut input: FlipSampleRequest) -> Result<Sample> {
    input.sample.validate(input.labels.len(), 16_777_216)?;
    let [channels, height, width] = input.sample.input.shape.as_slice() else {
        return Err(anyhow!(
            "Image augmentation requires [channels,height,width]"
        ));
    };
    let (channels, height, width) = (*channels, *height, *width);
    for channel in 0..channels {
        for row in 0..height {
            let offset = (channel * height + row) * width;
            input.sample.input.values[offset..offset + width].reverse();
        }
    }
    input.sample.annotation = input.sample.annotation.horizontal_flip();
    input.sample.id = format!(
        "{}:flip:{}",
        input.sample.id,
        flow_like_ml_core::content_digest(&flow_like_types::json::to_vec(&input.sample.input)?)
    );
    Ok(input.sample)
}
#[crate::register_node]
#[derive(Default)]
pub struct AugmentInspectionImageNode;
super::operation!(
    AugmentInspectionImageNode,
    "ml_augment_inspection_image",
    "Flip Inspection Image",
    "Flip pixels and box/mask annotations while preserving the source split group",
    "flipImage",
    "AI/ML/Vision",
    FlipSampleRequest,
    Sample,
    flip
);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rgb_channels_and_normalization_are_explicit() {
        let image = flow_like_types::image::DynamicImage::ImageRgb8(
            flow_like_types::image::RgbImage::from_raw(1, 1, vec![255, 0, 128]).unwrap(),
        );
        let result = preprocess(image, 1, 1, [0.5; 3], [0.5; 3]).unwrap();
        assert_eq!(result.tensor.shape, vec![3, 1, 1]);
        assert_eq!(&result.tensor.values[..2], &[1.0, -1.0]);
        assert!((result.tensor.values[2] - 1.0 / 255.0).abs() < 1e-6);
    }
}

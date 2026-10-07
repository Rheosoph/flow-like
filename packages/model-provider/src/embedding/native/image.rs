// Adapted from FastEmbed 5.17.2 (Apache-2.0), with direct ORT sessions and bounded batches.
// Attribution: thirdparty/manual-notices/fastembed-compatibility.md.
use super::{SessionOptions, output::image_vectors};
use anyhow::{Context, Result, anyhow, ensure};
use image::{DynamicImage, GenericImageView, imageops::FilterType};
use ndarray::{Array3, Array4, ArrayViewMut3, Axis};
use ort::{session::Session, value::TensorRef};
use serde_json::Value;
use std::{borrow::Cow, io::Cursor, path::Path};

/// Preprocessing contract for existing CLIP, ConvNext and BiT embedding Bits.
#[derive(Debug, Clone)]
pub struct ImagePreprocessor {
    resize: Option<(u32, u32)>,
    crop: Option<(u32, u32)>,
    rescale: Option<f32>,
    normalization: Option<([f32; 3], [f32; 3])>,
}

impl ImagePreprocessor {
    pub fn from_bytes(bytes: impl AsRef<[u8]>) -> Result<Self> {
        let config: Value = serde_json::from_slice(bytes.as_ref())
            .context("Invalid image preprocessor configuration")?;
        let mode = config["image_processor_type"]
            .as_str()
            .unwrap_or("CLIPImageProcessor");
        let (resize, crop) = match mode {
            "CLIPImageProcessor" | "BitImageProcessor" => {
                let resize = if config["do_resize"].as_bool().unwrap_or(false) {
                    let size = &config["size"];
                    if let Some(edge) = size["shortest_edge"].as_u64() {
                        Some((dimension(edge)?, dimension(edge)?))
                    } else {
                        // The legacy processor passes (height, width) to resize_exact.
                        // Retain that ordering for embeddings already stored in indexes.
                        Some((number(size, "height")?, number(size, "width")?))
                    }
                } else {
                    None
                };
                let crop = if config["do_center_crop"].as_bool().unwrap_or(false) {
                    let size = &config["crop_size"];
                    if let Some(edge) = size.as_u64() {
                        Some((dimension(edge)?, dimension(edge)?))
                    } else {
                        Some((number(size, "width")?, number(size, "height")?))
                    }
                } else {
                    None
                };
                (resize, crop)
            }
            "ConvNextFeatureExtractor" => {
                let edge = number(&config["size"], "shortest_edge")?;
                if edge < 384 {
                    let crop_pct = config["crop_pct"].as_f64().unwrap_or(0.875);
                    ensure!(
                        crop_pct.is_finite() && crop_pct > 0.0,
                        "crop_pct must be positive"
                    );
                    let resized = f64::from(edge) / crop_pct;
                    ensure!(
                        resized >= 1.0 && resized <= f64::from(u32::MAX),
                        "Invalid ConvNext resize dimensions"
                    );
                    (Some((resized as u32, resized as u32)), Some((edge, edge)))
                } else {
                    (Some((edge, edge)), None)
                }
            }
            _ => return Err(anyhow!("Unsupported legacy image processor {mode}")),
        };
        let rescale = config["do_rescale"]
            .as_bool()
            .unwrap_or(true)
            .then(|| config["rescale_factor"].as_f64().unwrap_or(1.0 / 255.0) as f32);
        if let Some(scale) = rescale {
            ensure!(scale.is_finite(), "rescale_factor must be finite");
        }
        let normalization = if config["do_normalize"].as_bool().unwrap_or(false) {
            let mean = channels(&config["image_mean"], "image_mean")?;
            let std = channels(&config["image_std"], "image_std")?;
            ensure!(
                std.iter().all(|value| *value > 0.0),
                "image_std must be positive"
            );
            Some((mean, std))
        } else {
            None
        };
        Ok(Self {
            resize,
            crop,
            rescale,
            normalization,
        })
    }

    pub fn preprocess(&self, image: DynamicImage) -> Result<Array3<f32>> {
        let image = self.prepare_image(Cow::Owned(DynamicImage::ImageRgb8(image.into_rgb8())))?;
        self.preprocess_prepared(&image)
    }

    pub fn preprocess_borrowed(&self, image: &DynamicImage) -> Result<Array3<f32>> {
        let image = self.prepare_image(Cow::Borrowed(image))?;
        self.preprocess_prepared(&image)
    }

    fn prepare_image<'a>(&self, mut image: Cow<'a, DynamicImage>) -> Result<Cow<'a, DynamicImage>> {
        if image.as_rgb8().is_none() {
            image = Cow::Owned(DynamicImage::ImageRgb8(image.to_rgb8()));
        }
        if let Some((width, height)) = self.resize {
            // Existing image vectors use a square stretch for shortest_edge, followed
            // by CatmullRom filtering. Aspect-preserving resizing needs a new space ID.
            image = Cow::Owned(image.resize_exact(width, height, FilterType::CatmullRom));
        }
        let (origin_width, origin_height) = image.dimensions();
        ensure!(
            origin_width > 0 && origin_height > 0,
            "Cannot embed an empty image"
        );
        Ok(image)
    }

    fn prepared_shape(&self, image: &DynamicImage) -> (usize, usize, usize) {
        let (width, height) = self.crop.unwrap_or_else(|| image.dimensions());
        (3, height as usize, width as usize)
    }

    fn preprocess_prepared(&self, image: &DynamicImage) -> Result<Array3<f32>> {
        let mut pixels = Array3::zeros(self.prepared_shape(image));
        self.write_pixels(image, pixels.view_mut())?;
        Ok(pixels)
    }

    fn write_pixels(&self, image: &DynamicImage, mut pixels: ArrayViewMut3<'_, f32>) -> Result<()> {
        ensure!(
            pixels.dim() == self.prepared_shape(image),
            "Images in a batch must have equal preprocessed dimensions"
        );
        let (origin_width, origin_height) = image.dimensions();
        let (width, height) = self.crop.unwrap_or((origin_width, origin_height));
        let (crop_x, crop_y, crop_width, crop_height) = if self.crop.is_some() {
            let crop_width = width.min(origin_width);
            let crop_height = height.min(origin_height);
            (
                (origin_width - crop_width) / 2,
                (origin_height - crop_height) / 2,
                crop_width,
                crop_height,
            )
        } else {
            (0, 0, origin_width, origin_height)
        };
        let offset_x = (width - crop_width) / 2;
        let offset_y = (height - crop_height) / 2;
        let image = image
            .as_rgb8()
            .ok_or_else(|| anyhow!("Prepared embedding image must be RGB8"))?;
        for y in 0..crop_height {
            for x in 0..crop_width {
                let pixel = image.get_pixel(x + crop_x, y + crop_y);
                for channel in 0..3 {
                    pixels[[channel, (y + offset_y) as usize, (x + offset_x) as usize]] =
                        f32::from(pixel[channel]);
                }
            }
        }
        if let Some(scale) = self.rescale {
            pixels *= scale;
        }
        if let Some((mean, std)) = self.normalization {
            for channel in 0..3 {
                pixels
                    .index_axis_mut(Axis(0), channel)
                    .mapv_inplace(|value| (value - mean[channel]) / std[channel]);
            }
        }
        Ok(())
    }

    pub fn preprocess_batch(&self, images: Vec<DynamicImage>) -> Result<Array4<f32>> {
        ensure!(!images.is_empty(), "Cannot preprocess an empty image batch");
        let mut images = images.into_iter();
        let first = self.prepare_image(Cow::Owned(DynamicImage::ImageRgb8(
            images.next().expect("nonempty image batch").into_rgb8(),
        )))?;
        let shape = self.prepared_shape(&first);
        let mut batch = Array4::zeros((images.len() + 1, shape.0, shape.1, shape.2));
        self.write_pixels(&first, batch.index_axis_mut(Axis(0), 0))?;
        for (index, image) in images.enumerate() {
            let image =
                self.prepare_image(Cow::Owned(DynamicImage::ImageRgb8(image.into_rgb8())))?;
            self.write_pixels(&image, batch.index_axis_mut(Axis(0), index + 1))?;
        }
        Ok(batch)
    }

    /// Fill the batch directly, borrowing RGB input buffers until resizing is needed.
    pub fn preprocess_batch_borrowed(&self, images: &[&DynamicImage]) -> Result<Array4<f32>> {
        ensure!(!images.is_empty(), "Cannot preprocess an empty image batch");
        let first = self.prepare_image(Cow::Borrowed(images[0]))?;
        let shape = self.prepared_shape(&first);
        let mut batch = Array4::zeros((images.len(), shape.0, shape.1, shape.2));
        self.write_pixels(&first, batch.index_axis_mut(Axis(0), 0))?;
        for (index, image) in images.iter().enumerate().skip(1) {
            let image = self.prepare_image(Cow::Borrowed(*image))?;
            self.write_pixels(&image, batch.index_axis_mut(Axis(0), index))?;
        }
        Ok(batch)
    }
}

fn dimension(value: u64) -> Result<u32> {
    let value = u32::try_from(value).context("Image dimension exceeds u32")?;
    ensure!(value > 0, "Image dimensions must be positive");
    Ok(value)
}

fn number(config: &Value, name: &str) -> Result<u32> {
    dimension(
        config[name]
            .as_u64()
            .ok_or_else(|| anyhow!("Missing image {name}"))?,
    )
}

fn channels(value: &Value, name: &str) -> Result<[f32; 3]> {
    let values = value.as_array().ok_or_else(|| anyhow!("Missing {name}"))?;
    ensure!(values.len() == 3, "{name} must have three channels");
    let mut result = [0.0; 3];
    for (slot, value) in result.iter_mut().zip(values) {
        *slot = value
            .as_f64()
            .ok_or_else(|| anyhow!("{name} must contain numbers"))? as f32;
        ensure!(slot.is_finite(), "{name} must contain finite numbers");
    }
    Ok(result)
}

pub struct NativeImageEmbedding {
    preprocessor: ImagePreprocessor,
    session: Session,
    input_name: String,
}

impl NativeImageEmbedding {
    pub fn new_from_file(
        path: impl AsRef<Path>,
        preprocessor: impl AsRef<[u8]>,
        options: SessionOptions,
    ) -> Result<Self> {
        Self::from_session(options.load_file(path)?, preprocessor)
    }

    pub fn new_from_memory(
        graph: &[u8],
        preprocessor: impl AsRef<[u8]>,
        options: SessionOptions,
    ) -> Result<Self> {
        Self::from_session(options.load_memory(graph)?, preprocessor)
    }

    pub fn from_session(session: Session, preprocessor: impl AsRef<[u8]>) -> Result<Self> {
        ensure!(
            session.inputs().len() == 1,
            "Image adapter requires a single-input vision graph"
        );
        let input_name = session.inputs()[0].name().to_owned();
        Ok(Self {
            session,
            input_name,
            preprocessor: ImagePreprocessor::from_bytes(preprocessor)?,
        })
    }

    pub fn preprocessor(&self) -> &ImagePreprocessor {
        &self.preprocessor
    }

    pub fn output_dimensions(&self) -> Option<usize> {
        let outputs = self.session.outputs();
        let output = if outputs.len() == 1 {
            outputs.first()
        } else {
            outputs
                .iter()
                .find(|output| output.name() == "image_embeds")
                .or_else(|| {
                    outputs
                        .iter()
                        .find(|output| output.name() == "last_hidden_state")
                })
        }?;
        let dimensions = *output.dtype().tensor_shape()?.last()?;
        usize::try_from(dimensions)
            .ok()
            .filter(|dimensions| *dimensions > 0)
    }

    pub fn embed_images(&mut self, images: Vec<DynamicImage>) -> Result<Vec<Vec<f32>>> {
        if images.is_empty() {
            return Ok(Vec::new());
        }
        let pixels = self.preprocessor.preprocess_batch(images)?;
        self.embed_pixels(&pixels)
    }

    pub fn embed_pixels(&mut self, pixels: &Array4<f32>) -> Result<Vec<Vec<f32>>> {
        if pixels.shape()[0] == 0 {
            return Ok(Vec::new());
        }
        let outputs = self.session.run(ort::inputs![
            self.input_name.as_str() => TensorRef::from_array_view(pixels)?,
        ])?;
        let vectors = image_vectors(&outputs)?;
        ensure!(
            vectors.len() == pixels.shape()[0],
            "Embedding graph changed image batch size"
        );
        Ok(vectors)
    }

    pub fn embed_bytes(
        &mut self,
        images: &[&[u8]],
        batch_size: Option<usize>,
    ) -> Result<Vec<Vec<f32>>> {
        let batch_size = batch_size.unwrap_or(256);
        ensure!(batch_size > 0, "batch_size must be greater than zero");
        let mut embeddings = Vec::with_capacity(images.len());
        for batch in images.chunks(batch_size) {
            let decoded = batch
                .iter()
                .map(|bytes| {
                    image::ImageReader::new(Cursor::new(bytes))
                        .with_guessed_format()?
                        .decode()
                        .map_err(Into::into)
                })
                .collect::<Result<Vec<_>>>()?;
            embeddings.extend(self.embed_images(decoded)?);
        }
        Ok(embeddings)
    }

    pub fn embed<P: AsRef<Path>>(
        &mut self,
        images: impl AsRef<[P]>,
        batch_size: Option<usize>,
    ) -> Result<Vec<Vec<f32>>> {
        let batch_size = batch_size.unwrap_or(256);
        ensure!(batch_size > 0, "batch_size must be greater than zero");
        let images = images.as_ref();
        let mut embeddings = Vec::with_capacity(images.len());
        for batch in images.chunks(batch_size) {
            let decoded = batch
                .iter()
                .map(|path| image::ImageReader::open(path)?.decode().map_err(Into::into))
                .collect::<Result<Vec<_>>>()?;
            embeddings.extend(self.embed_images(decoded)?);
        }
        Ok(embeddings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    // Retain the allocating crop-and-stack path as a pixel oracle for borrowed preprocessing.
    fn reference_pixels(preprocessor: &ImagePreprocessor, image: DynamicImage) -> Array3<f32> {
        let mut image = DynamicImage::ImageRgb8(image.into_rgb8());
        if let Some((width, height)) = preprocessor.resize {
            image = image.resize_exact(width, height, FilterType::CatmullRom);
        }
        let (origin_width, origin_height) = image.dimensions();
        let (width, height) = preprocessor.crop.unwrap_or((origin_width, origin_height));
        if preprocessor.crop.is_some() {
            let crop_width = width.min(origin_width);
            let crop_height = height.min(origin_height);
            image = image.crop_imm(
                (origin_width - crop_width) / 2,
                (origin_height - crop_height) / 2,
                crop_width,
                crop_height,
            );
        }
        let offset_x = (width - image.width()) / 2;
        let offset_y = (height - image.height()) / 2;
        let mut pixels = Array3::zeros((3, height as usize, width as usize));
        for (x, y, pixel) in image.to_rgb8().enumerate_pixels() {
            for channel in 0..3 {
                pixels[[channel, (y + offset_y) as usize, (x + offset_x) as usize]] =
                    f32::from(pixel[channel]);
            }
        }
        if let Some(scale) = preprocessor.rescale {
            pixels *= scale;
        }
        if let Some((mean, std)) = preprocessor.normalization {
            for channel in 0..3 {
                pixels
                    .index_axis_mut(Axis(0), channel)
                    .mapv_inplace(|value| (value - mean[channel]) / std[channel]);
            }
        }
        pixels
    }

    #[test]
    fn borrowed_images_match_allocating_reference_pixels_exactly() {
        let images = vec![
            DynamicImage::ImageRgb8(RgbImage::from_fn(7, 5, |x, y| {
                Rgb([(x * 31) as u8, (y * 41) as u8, ((x + y) * 17) as u8])
            })),
            DynamicImage::ImageRgba8(image::RgbaImage::from_fn(7, 5, |x, y| {
                image::Rgba([(y * 37) as u8, (x * 21) as u8, 199, 83])
            })),
            DynamicImage::ImageLuma8(image::GrayImage::from_fn(7, 5, |x, y| {
                image::Luma([((x + y) * 23) as u8])
            })),
            DynamicImage::ImageRgba32F(image::Rgba32FImage::from_fn(7, 5, |x, y| {
                image::Rgba([x as f32 / 7.0, y as f32 / 5.0, 0.51, 0.3])
            })),
        ];
        for config in [
            serde_json::json!({"do_rescale":false}),
            serde_json::json!({
                "do_resize":true,"size":{"shortest_edge":4},
                "do_center_crop":true,"crop_size":{"width":3,"height":2}
            }),
            serde_json::json!({
                "do_center_crop":true,"crop_size":{"width":10,"height":8},
                "do_normalize":true,"image_mean":[0.48,0.45,0.4],"image_std":[0.2,0.3,0.4]
            }),
            serde_json::json!({
                "image_processor_type":"ConvNextFeatureExtractor","size":{"shortest_edge":8},
                "crop_pct":0.875,"do_normalize":true,
                "image_mean":[0.5,0.4,0.3],"image_std":[0.25,0.2,0.15]
            }),
        ] {
            let preprocessor =
                ImagePreprocessor::from_bytes(serde_json::to_vec(&config).unwrap()).unwrap();
            let borrowed = images.iter().collect::<Vec<_>>();
            let actual = preprocessor.preprocess_batch_borrowed(&borrowed).unwrap();
            assert_eq!(
                actual,
                preprocessor.preprocess_batch(images.clone()).unwrap()
            );
            for (index, image) in images.iter().enumerate() {
                let reference = reference_pixels(&preprocessor, image.clone());
                assert_eq!(actual.index_axis(Axis(0), index), reference.view());
                assert_eq!(preprocessor.preprocess_borrowed(image).unwrap(), reference);
                assert_eq!(preprocessor.preprocess(image.clone()).unwrap(), reference);
            }
        }
    }

    #[test]
    fn borrowed_preprocessing_checks_empty_and_inconsistent_shapes() {
        let preprocessor = ImagePreprocessor::from_bytes(br#"{"do_rescale":false}"#).unwrap();
        assert!(preprocessor.preprocess_batch_borrowed(&[]).is_err());
        let image = DynamicImage::ImageRgb8(RgbImage::new(2, 2));
        let prepared = preprocessor.prepare_image(Cow::Borrowed(&image)).unwrap();
        assert!(matches!(prepared, Cow::Borrowed(_)));
        let other = DynamicImage::ImageRgb8(RgbImage::new(3, 2));
        assert!(
            preprocessor
                .preprocess_batch_borrowed(&[&image, &other])
                .is_err()
        );
        let empty = DynamicImage::ImageRgb8(RgbImage::new(0, 0));
        assert!(preprocessor.preprocess_borrowed(&empty).is_err());
    }

    #[test]
    fn preprocessing_keeps_legacy_square_resize_and_channel_order() {
        let preprocessor = ImagePreprocessor::from_bytes(
            br#"{
            "do_resize": true, "size": {"shortest_edge": 2},
            "do_normalize": true, "image_mean": [0.5,0,0], "image_std": [0.5,1,1]
        }"#,
        )
        .unwrap();
        let pixels = preprocessor
            .preprocess(DynamicImage::ImageRgb8(RgbImage::from_pixel(
                4,
                2,
                Rgb([255, 0, 128]),
            )))
            .unwrap();
        assert_eq!(pixels.dim(), (3, 2, 2));
        assert_eq!(pixels[[0, 0, 0]], 1.0);
        assert_eq!(pixels[[1, 0, 0]], 0.0);
        assert_eq!(pixels[[2, 0, 0]], 128.0 * (1.0 / 255.0));
    }

    #[test]
    fn center_crop_pads_before_rescaling_and_normalization() {
        let preprocessor = ImagePreprocessor::from_bytes(
            br#"{
            "do_center_crop": true, "crop_size": {"height":4,"width":6},
            "do_rescale": false
        }"#,
        )
        .unwrap();
        let pixels = preprocessor
            .preprocess(DynamicImage::ImageRgb8(RgbImage::from_pixel(
                2,
                2,
                Rgb([1, 2, 3]),
            )))
            .unwrap();
        assert_eq!(pixels.dim(), (3, 4, 6));
        assert_eq!(pixels[[0, 1, 2]], 1.);
        assert_eq!(pixels[[2, 2, 3]], 3.);
        assert_eq!(pixels[[0, 0, 0]], 0.);
    }

    #[test]
    fn unknown_processors_and_invalid_normalization_are_rejected() {
        assert!(ImagePreprocessor::from_bytes(br#"{"image_processor_type":"Unknown"}"#).is_err());
        assert!(
            ImagePreprocessor::from_bytes(
                br#"{"do_normalize":true,"image_mean":[0,0,0],"image_std":[1,0,1]}"#
            )
            .is_err()
        );
    }

    #[test]
    fn encoded_image_formats_remain_available_without_the_reference_backend() {
        let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(2, 2, Rgb([1, 2, 3])));
        let mut encoded = Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let decoded = image::ImageReader::new(Cursor::new(encoded.into_inner()))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!(decoded.to_rgb8(), image.to_rgb8());
    }
}

use anyhow::{Result, ensure};
use image::{DynamicImage, RgbImage};

pub(super) const PATCH_SIZE: usize = 16;
pub(super) const PATCH_VALUES: usize = PATCH_SIZE * PATCH_SIZE * 3;
const POOL_SIZE: usize = 3;

pub(super) struct VisionFeatures {
    pub pixels: Vec<f32>,
    pub positions: Vec<i64>,
    pub patches: usize,
    pub tokens: usize,
}

pub(super) fn token_count(image: &DynamicImage, budget: usize) -> Result<usize> {
    let (width, height) = target_size(image.width() as usize, image.height() as usize, budget)?;
    Ok(width / PATCH_SIZE * (height / PATCH_SIZE) / (POOL_SIZE * POOL_SIZE))
}

// Match Gemma4ImageProcessor's aspect-ratio resize and row-major RGB patch layout.
pub(super) fn preprocess(image: &DynamicImage, budget: usize) -> Result<VisionFeatures> {
    ensure!(
        [70, 140, 280, 560, 1120].contains(&budget),
        "Unsupported vision token budget {budget}"
    );
    let (width, height) = target_size(image.width() as usize, image.height() as usize, budget)?;
    let rgb = image.to_rgb8();
    let resized = resize_bicubic(&rgb, width, height);
    let cols = width / PATCH_SIZE;
    let rows = height / PATCH_SIZE;
    let patches = budget * POOL_SIZE * POOL_SIZE;
    let mut pixels = vec![0.0; patches * PATCH_VALUES];
    let mut positions = vec![-1; patches * 2];
    for row in 0..rows {
        for col in 0..cols {
            let patch = row * cols + col;
            positions[patch * 2] = col as i64;
            positions[patch * 2 + 1] = row as i64;
            for y in 0..PATCH_SIZE {
                for x in 0..PATCH_SIZE {
                    for channel in 0..3 {
                        let source =
                            ((row * PATCH_SIZE + y) * width + col * PATCH_SIZE + x) * 3 + channel;
                        let target = patch * PATCH_VALUES + (y * PATCH_SIZE + x) * 3 + channel;
                        pixels[target] = resized[source] as f32 * (1.0 / 255.0);
                    }
                }
            }
        }
    }
    Ok(VisionFeatures {
        pixels,
        positions,
        patches,
        tokens: rows * cols / (POOL_SIZE * POOL_SIZE),
    })
}

fn target_size(width: usize, height: usize, budget: usize) -> Result<(usize, usize)> {
    ensure!(width > 0 && height > 0, "Cannot embed an empty image");
    let multiple = PATCH_SIZE * POOL_SIZE;
    let target_pixels = budget * multiple * multiple;
    let scale = (target_pixels as f64 / (width as f64 * height as f64)).sqrt();
    let mut out_width = (width as f64 * scale / multiple as f64).floor() as usize * multiple;
    let mut out_height = (height as f64 * scale / multiple as f64).floor() as usize * multiple;
    if out_height == 0 {
        out_height = multiple;
        out_width = ((width / height) * multiple).min(budget * multiple);
    } else if out_width == 0 {
        out_width = multiple;
        out_height = ((height / width) * multiple).min(budget * multiple);
    }
    ensure!(
        out_width * out_height <= target_pixels,
        "Image resize exceeds its patch budget"
    );
    Ok((out_width, out_height))
}

// Antialiased cubic convolution with pixel centers and edge-normalized weights.
// The a=-0.5 kernel matches the reference processor's bicubic antialias mode.
fn coefficients(source: usize, target: usize) -> Vec<(usize, Vec<f64>)> {
    let scale = source as f64 / target as f64;
    let filter_scale = scale.max(1.0);
    let support = 2.0 * filter_scale;
    (0..target)
        .map(|out| {
            let center = (out as f64 + 0.5) * scale;
            let start = (center - support + 0.5).floor().max(0.0) as usize;
            let end = (center + support + 0.5).floor().min(source as f64) as usize;
            let mut weights: Vec<f64> = (start..end)
                .map(|input| {
                    let x = ((input as f64 + 0.5 - center) / filter_scale).abs();
                    if x < 1.0 {
                        ((1.5 * x - 2.5) * x) * x + 1.0
                    } else if x < 2.0 {
                        ((-0.5 * x + 2.5) * x - 4.0) * x + 2.0
                    } else {
                        0.0
                    }
                })
                .collect();
            let sum: f64 = weights.iter().sum();
            for weight in &mut weights {
                *weight /= sum;
            }
            (start, weights)
        })
        .collect()
}

fn integer_coefficients(source: usize, target: usize) -> (Vec<(usize, Vec<i64>)>, u32) {
    let coefficients = coefficients(source, target);
    let maximum = coefficients
        .iter()
        .flat_map(|(_, weights)| weights)
        .copied()
        .fold(0.0, f64::max);
    let precision = (0..22)
        .find(|bits| (0.5 + maximum * ((1_u64 << (bits + 1)) as f64)) as i64 >= 1 << 15)
        .unwrap_or(22);
    let scale = (1_u64 << precision) as f64;
    let coefficients = coefficients
        .into_iter()
        .map(|(start, weights)| {
            (
                start,
                weights
                    .into_iter()
                    .map(|weight| (weight * scale).round() as i64)
                    .collect(),
            )
        })
        .collect();
    (coefficients, precision)
}

fn resize_bicubic(image: &RgbImage, width: usize, height: usize) -> Vec<u8> {
    let source_width = image.width() as usize;
    let source_height = image.height() as usize;
    if source_width == width && source_height == height {
        return image.as_raw().clone();
    }
    // The reference's uint8 resize quantizes each axis's weights to signed
    // 16-bit coefficients, then rounds and clamps after each separable pass.
    let (horizontal, horizontal_precision) = integer_coefficients(source_width, width);
    let (vertical, vertical_precision) = integer_coefficients(source_height, height);
    let mut intermediate = vec![0_u8; source_height * width * 3];
    for y in 0..source_height {
        for (x, (start, weights)) in horizontal.iter().enumerate() {
            for channel in 0..3 {
                let sum: i64 = weights
                    .iter()
                    .enumerate()
                    .map(|(offset, weight)| {
                        image.as_raw()[(y * source_width + start + offset) * 3 + channel] as i64
                            * weight
                    })
                    .sum();
                intermediate[(y * width + x) * 3 + channel] =
                    ((sum + (1 << (horizontal_precision - 1))) >> horizontal_precision)
                        .clamp(0, 255) as u8;
            }
        }
    }
    let mut output = vec![0_u8; height * width * 3];
    for (y, (start, weights)) in vertical.iter().enumerate() {
        for x in 0..width {
            for channel in 0..3 {
                let sum: i64 = weights
                    .iter()
                    .enumerate()
                    .map(|(offset, weight)| {
                        intermediate[((start + offset) * width + x) * 3 + channel] as i64 * weight
                    })
                    .sum();
                output[(y * width + x) * 3 + channel] = ((sum + (1 << (vertical_precision - 1)))
                    >> vertical_precision)
                    .clamp(0, 255) as u8;
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_aspect_ratios_and_padding() {
        assert_eq!(target_size(640, 480, 280).unwrap(), (912, 672));
        assert_eq!(target_size(1, 10000, 280).unwrap(), (48, 13440));
        let image =
            DynamicImage::ImageRgb8(RgbImage::from_pixel(16, 16, image::Rgb([255, 128, 0])));
        let features = preprocess(&image, 70).unwrap();
        assert_eq!(features.tokens, 64);
        assert_eq!(features.positions[0..4], [0, 0, 1, 0]);
        assert_eq!(features.positions[64 * 9 * 2], -1);
        assert_eq!(features.pixels[0], 1.0);
        assert_eq!(features.pixels[2], 0.0);
    }
}

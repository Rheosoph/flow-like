use std::io::Cursor;

use flow_like_catalog_media_image::image::content::read_barcodes::{
    Barcode, BarcodePreprocessingOptions, ReadBarcodeOptions, read_barcodes_in_luma,
};
use flow_like_types::image::{
    DynamicImage, GrayImage, ImageDecoder, ImageReader, Limits,
    imageops::{self, FilterType},
};
use flow_like_types::{Bytes, Result};

use super::{BarcodeReader, BarcodeRequest, BarcodeScan, CodeBox, FoundCode};
use crate::error::{ToolError, code};

/// Larger images are scaled down first: every preprocessing variant is a full-size copy.
const MAX_PIXELS: u64 = 16_000_000;
const MAX_EDGE: u32 = 20_000;

/// The Read Barcodes node's decoder (rxing).
pub struct CatalogBarcodes;

fn unsupported(why: impl std::fmt::Display) -> flow_like_types::Error {
    ToolError::new(
        code::UNSUPPORTED_FILE,
        format!("The file is not an image this host can read: {why}"),
    )
    .into()
}

fn decode(bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(unsupported)?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(unsupported)?;
    let orientation = decoder.orientation().map_err(unsupported)?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(unsupported)?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// The size to read a `width`×`height` image at when it holds more than [`MAX_PIXELS`].
pub(super) fn fitted_size(width: u32, height: u32) -> Option<(u32, u32)> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels <= MAX_PIXELS {
        return None;
    }
    let factor = (MAX_PIXELS as f64 / pixels as f64).sqrt();
    let scaled = |edge: u32| ((f64::from(edge) * factor).floor() as u32).max(1);
    Some((scaled(width), scaled(height)))
}

fn fit(gray: GrayImage) -> GrayImage {
    match fitted_size(gray.width(), gray.height()) {
        Some((width, height)) => imageops::resize(&gray, width, height, FilterType::Triangle),
        None => gray,
    }
}

pub(super) fn options(request: &BarcodeRequest) -> ReadBarcodeOptions {
    let mut options = ReadBarcodeOptions {
        expected_formats: request
            .formats
            .iter()
            .map(|kind| kind.as_str().to_ascii_uppercase())
            .collect(),
        max_results: Some(request.max_codes.max(1)),
        ..ReadBarcodeOptions::default()
    };
    if request.thorough {
        options.try_harder = true;
        options.preprocess = "Aggressive".to_string();
        options.preprocessing = BarcodePreprocessingOptions {
            local_contrast: true,
            sharpen: true,
            adaptive_threshold: true,
            upscale_factor: 2,
            max_variants: 16,
            max_decode_attempts: 32,
            ..BarcodePreprocessingOptions::default()
        };
    }
    options
}

fn found(code: &Barcode, scale_x: f32, scale_y: f32) -> FoundCode {
    let bounds = code.bounds().map(|[left, top, right, bottom]| {
        let left = (left * scale_x).max(0.0);
        let top = (top * scale_y).max(0.0);
        CodeBox {
            x: left.round() as u32,
            y: top.round() as u32,
            width: ((right * scale_x) - left).max(0.0).round() as u32,
            height: ((bottom * scale_y) - top).max(0.0).round() as u32,
        }
    });
    FoundCode {
        format: code.format_name().to_ascii_lowercase(),
        text: code.text().to_string(),
        bounds,
    }
}

impl BarcodeReader for CatalogBarcodes {
    fn read(&self, image: Bytes, request: &BarcodeRequest) -> Result<BarcodeScan> {
        let gray = decode(&image)?.to_luma8();
        let (width, height) = gray.dimensions();
        let gray = fit(gray);
        let (seen_width, seen_height) = gray.dimensions();
        let scale_x = width as f32 / seen_width as f32;
        let scale_y = height as f32 / seen_height as f32;
        let mut scan = BarcodeScan {
            width,
            height,
            scaled_to: (seen_width != width).then_some((seen_width, seen_height)),
            ..BarcodeScan::default()
        };
        // The options are built here and always valid, so an error means a symbol was found
        // that would not decode.
        match read_barcodes_in_luma(gray.into_raw(), seen_width, seen_height, &options(request)) {
            Ok(codes) => {
                scan.codes = codes
                    .iter()
                    .map(|code| found(code, scale_x, scale_y))
                    .collect();
            }
            Err(error) => scan.unreadable = Some(format!("{error:#}")),
        }
        Ok(scan)
    }
}

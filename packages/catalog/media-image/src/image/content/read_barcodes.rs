use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{anyhow, async_trait, json::json};
#[cfg(feature = "execute")]
use rayon::prelude::*;
use rxing::{BarcodeFormat, DecodeHints, RXingResult};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

mod scan;

use scan::{Bounds, Scanner, is_linear};

const READABLE_BARCODE_FORMATS: &[&str] = &[
    "AZTEC",
    "CODABAR",
    "CODE_39",
    "CODE_93",
    "CODE_128",
    "DATA_MATRIX",
    "EAN_8",
    "EAN_13",
    "ITF",
    "MAXICODE",
    "PDF_417",
    "QR_CODE",
    "MICRO_QR_CODE",
    "RECTANGULAR_MICRO_QR_CODE",
    "RSS_14",
    "RSS_EXPANDED",
    "TELEPEN",
    "UPC_A",
    "UPC_E",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreprocessMode {
    None,
    Balanced,
    Fallback,
    Aggressive,
    Industrial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BarcodePolarity {
    Auto,
    DarkOnLight,
    LightOnDark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecodeRotation {
    Deg0,
    Deg90,
    Deg180,
    Deg270,
}

#[derive(Clone)]
struct DecodeVariant {
    luma: Vec<u8>,
    width: u32,
    height: u32,
    hash: u64,
    point_scale: f32,
    x_offset: u32,
    y_offset: u32,
    base_width: u32,
    base_height: u32,
    rotation: DecodeRotation,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BarcodePoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Barcode {
    text: String,
    raw_bytes: Vec<u8>,
    num_bits: usize,
    format: String,
    timestamp: u128,
    // Rows of a linear code that read the same text. Plain comments: doc comments would change
    // the pin schema.
    line_count: usize,
    // A linear code has the four corners of its bars, clockwise from the top left as read.
    points: Vec<BarcodePoint>,
    #[serde(skip)]
    #[schemars(skip)]
    bounds: Option<Bounds>,
    #[serde(skip)]
    #[schemars(skip)]
    linear: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ReadBarcodeOptions {
    #[serde(default)]
    #[schemars(schema_with = "bool_false_schema")]
    pub filter: bool,
    #[serde(default = "default_barcode_format")]
    #[schemars(schema_with = "barcode_format_schema")]
    pub format: String,
    #[serde(default)]
    #[schemars(schema_with = "barcode_formats_array_schema")]
    pub expected_formats: Vec<String>,
    #[serde(default = "default_try_harder")]
    #[schemars(schema_with = "bool_false_schema")]
    pub try_harder: bool,
    #[serde(default = "default_also_inverted")]
    #[schemars(schema_with = "bool_true_schema")]
    pub also_inverted: bool,
    #[serde(default)]
    #[schemars(schema_with = "bool_false_schema")]
    pub pure_barcode: bool,
    /// Stop after this many validated results. Leave unset to search for every code.
    #[serde(default)]
    #[schemars(schema_with = "max_results_schema")]
    pub max_results: Option<usize>,
    #[serde(default = "default_preprocess")]
    #[schemars(schema_with = "barcode_preprocess_schema")]
    pub preprocess: String,
    #[serde(default)]
    pub validation: BarcodeValidationOptions,
    #[serde(default)]
    #[schemars(default = "schema_default_preprocessing")]
    pub preprocessing: BarcodePreprocessingOptions,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BarcodePreprocessingOptions {
    #[serde(default = "default_polarity")]
    #[schemars(schema_with = "barcode_polarity_schema")]
    pub polarity: String,
    #[serde(default)]
    pub roi: Option<BarcodeRoi>,
    #[serde(default)]
    #[schemars(schema_with = "barcode_rotations_schema")]
    pub rotations: Vec<u16>,
    #[serde(default = "default_true")]
    #[schemars(schema_with = "bool_true_schema")]
    pub contrast_stretch: bool,
    #[serde(default)]
    #[schemars(schema_with = "bool_false_schema")]
    pub local_contrast: bool,
    #[serde(default)]
    #[schemars(schema_with = "bool_false_schema")]
    pub denoise: bool,
    #[serde(default)]
    #[schemars(schema_with = "bool_false_schema")]
    pub sharpen: bool,
    #[serde(default = "default_true")]
    #[schemars(schema_with = "bool_true_schema")]
    pub otsu_threshold: bool,
    #[serde(default)]
    #[schemars(schema_with = "bool_false_schema")]
    pub adaptive_threshold: bool,
    #[serde(default)]
    #[schemars(schema_with = "bool_false_schema")]
    pub morphology: bool,
    #[serde(default = "default_local_contrast_tile_size")]
    #[schemars(schema_with = "local_contrast_tile_size_schema")]
    pub local_contrast_tile_size: u32,
    #[serde(default = "default_adaptive_threshold_window")]
    #[schemars(schema_with = "adaptive_threshold_window_schema")]
    pub adaptive_threshold_window: u32,
    #[serde(default = "default_adaptive_threshold_bias")]
    #[schemars(schema_with = "adaptive_threshold_bias_schema")]
    pub adaptive_threshold_bias: i16,
    #[serde(default = "default_upscale_factor")]
    #[schemars(schema_with = "upscale_factor_schema")]
    pub upscale_factor: u32,
    #[serde(default = "default_max_upscale_pixels")]
    #[schemars(schema_with = "max_upscale_pixels_schema")]
    pub max_upscale_pixels: u64,
    #[serde(default = "default_max_preprocess_variants")]
    #[schemars(schema_with = "max_variants_schema")]
    pub max_variants: usize,
    #[serde(default = "default_max_decode_attempts")]
    #[schemars(schema_with = "max_decode_attempts_schema")]
    pub max_decode_attempts: usize,
    #[serde(default = "default_decode_threads")]
    #[schemars(
        schema_with = "decode_threads_schema",
        default = "schema_default_decode_threads"
    )]
    pub decode_threads: usize,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BarcodeRoi {
    #[schemars(schema_with = "roi_offset_schema")]
    pub x: u32,
    #[schemars(schema_with = "roi_offset_schema")]
    pub y: u32,
    #[schemars(schema_with = "roi_size_schema")]
    pub width: u32,
    #[schemars(schema_with = "roi_size_schema")]
    pub height: u32,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct BarcodeValidationOptions {
    #[serde(default)]
    pub min_text_length: Option<usize>,
    #[serde(default)]
    pub max_text_length: Option<usize>,
    #[serde(default)]
    pub text_prefix: Option<String>,
    #[serde(default)]
    pub text_suffix: Option<String>,
    #[serde(default)]
    pub text_contains: Option<String>,
    #[serde(default)]
    pub allowed_texts: Vec<String>,
}

impl Default for ReadBarcodeOptions {
    fn default() -> Self {
        Self {
            filter: false,
            format: default_barcode_format(),
            expected_formats: Vec::new(),
            try_harder: default_try_harder(),
            also_inverted: default_also_inverted(),
            pure_barcode: false,
            max_results: None,
            preprocess: default_preprocess(),
            validation: BarcodeValidationOptions::default(),
            preprocessing: BarcodePreprocessingOptions::default(),
        }
    }
}

impl Default for BarcodePreprocessingOptions {
    fn default() -> Self {
        Self {
            polarity: default_polarity(),
            roi: None,
            rotations: Vec::new(),
            contrast_stretch: true,
            local_contrast: false,
            denoise: false,
            sharpen: false,
            otsu_threshold: true,
            adaptive_threshold: false,
            morphology: false,
            local_contrast_tile_size: default_local_contrast_tile_size(),
            adaptive_threshold_window: default_adaptive_threshold_window(),
            adaptive_threshold_bias: default_adaptive_threshold_bias(),
            upscale_factor: default_upscale_factor(),
            max_upscale_pixels: default_max_upscale_pixels(),
            max_variants: default_max_preprocess_variants(),
            max_decode_attempts: default_max_decode_attempts(),
            decode_threads: default_decode_threads(),
        }
    }
}

fn effective_preprocessing_options(
    preprocess_mode: PreprocessMode,
    preprocessing: &BarcodePreprocessingOptions,
) -> BarcodePreprocessingOptions {
    let mut effective = preprocessing.clone();

    match preprocess_mode {
        PreprocessMode::None => {
            effective.contrast_stretch = false;
            effective.local_contrast = false;
            effective.denoise = false;
            effective.sharpen = false;
            effective.otsu_threshold = false;
            effective.adaptive_threshold = false;
            effective.morphology = false;
            effective.upscale_factor = 1;
            effective.max_variants = effective.max_variants.min(4);
            effective.max_decode_attempts = effective.max_decode_attempts.min(4);
        }
        PreprocessMode::Balanced => {
            effective.contrast_stretch = true;
            effective.local_contrast = false;
            effective.denoise = false;
            // Photos are rarely as sharp as their codes need.
            effective.sharpen = true;
            effective.otsu_threshold = true;
            effective.adaptive_threshold = false;
            effective.morphology = false;
            effective.upscale_factor = effective.upscale_factor.min(1);
            effective.max_variants = effective.max_variants.min(12);
            effective.max_decode_attempts = effective.max_decode_attempts.min(24);
        }
        PreprocessMode::Fallback => {
            effective.local_contrast = false;
            effective.denoise = false;
            effective.sharpen = false;
            effective.adaptive_threshold = false;
            effective.morphology = false;
            effective.upscale_factor = effective.upscale_factor.min(1);
            effective.max_variants = effective.max_variants.min(8);
            effective.max_decode_attempts = effective.max_decode_attempts.min(8);
        }
        PreprocessMode::Aggressive => {
            effective.max_variants = effective.max_variants.min(48);
            effective.max_decode_attempts = effective.max_decode_attempts.min(96);
        }
        PreprocessMode::Industrial => {}
    }

    effective
}

fn default_barcode_format() -> String {
    "QR_CODE".to_string()
}

fn default_polarity() -> String {
    "Auto".to_string()
}

fn default_true() -> bool {
    true
}

fn default_try_harder() -> bool {
    false
}

fn default_also_inverted() -> bool {
    true
}

fn default_preprocess() -> String {
    "Balanced".to_string()
}

fn barcode_preprocess_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "enum": ["None", "Balanced", "Fallback", "Aggressive", "Industrial"],
        "default": "Balanced"
    })
}

fn barcode_format_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "enum": ["AZTEC", "CODABAR", "CODE_39", "CODE_93", "CODE_128", "DATA_MATRIX", "EAN_8", "EAN_13", "ITF", "MAXICODE", "PDF_417", "QR_CODE", "MICRO_QR_CODE", "RECTANGULAR_MICRO_QR_CODE", "RSS_14", "RSS_EXPANDED", "TELEPEN", "UPC_A", "UPC_E"],
        "default": "QR_CODE"
    })
}

fn barcode_formats_array_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "array",
        "items": {
            "type": "string",
            "enum": ["AZTEC", "CODABAR", "CODE_39", "CODE_93", "CODE_128", "DATA_MATRIX", "EAN_8", "EAN_13", "ITF", "MAXICODE", "PDF_417", "QR_CODE", "MICRO_QR_CODE", "RECTANGULAR_MICRO_QR_CODE", "RSS_14", "RSS_EXPANDED", "TELEPEN", "UPC_A", "UPC_E"]
        },
        "uniqueItems": true,
        "default": []
    })
}

fn barcode_polarity_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "string",
        "enum": ["Auto", "DarkOnLight", "LightOnDark"],
        "default": "Auto"
    })
}

fn barcode_rotations_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "array",
        "items": {
            "type": "integer",
            "enum": [0, 90, 180, 270]
        },
        "uniqueItems": true,
        "default": []
    })
}

fn bool_true_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "boolean",
        "default": true
    })
}

fn bool_false_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "boolean",
        "default": false
    })
}

fn max_results_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["integer", "null"],
        "minimum": 1,
        "default": null
    })
}

fn local_contrast_tile_size_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "integer",
        "minimum": 8,
        "maximum": 1024,
        "default": 64
    })
}

fn adaptive_threshold_window_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "integer",
        "minimum": 3,
        "maximum": 255,
        "default": 31
    })
}

fn adaptive_threshold_bias_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "integer",
        "minimum": -255,
        "maximum": 255,
        "default": 5
    })
}

fn upscale_factor_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "integer",
        "minimum": 1,
        "maximum": 4,
        "default": 1
    })
}

fn max_upscale_pixels_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "integer",
        "minimum": 1000000,
        "maximum": 100000000,
        "default": 16000000
    })
}

fn max_variants_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "integer",
        "minimum": 1,
        "maximum": 128,
        "default": 12
    })
}

fn max_decode_attempts_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "integer",
        "minimum": 1,
        "maximum": 256,
        "default": 24
    })
}

fn decode_threads_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "integer",
        "minimum": 1,
        "maximum": 32,
        "default": 4
    })
}

fn roi_offset_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "integer",
        "minimum": 0,
        "default": 0
    })
}

fn roi_size_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": "integer",
        "minimum": 1
    })
}

fn default_local_contrast_tile_size() -> u32 {
    64
}

fn default_adaptive_threshold_window() -> u32 {
    31
}

fn default_adaptive_threshold_bias() -> i16 {
    5
}

fn default_upscale_factor() -> u32 {
    1
}

fn default_max_upscale_pixels() -> u64 {
    16_000_000
}

fn default_max_preprocess_variants() -> usize {
    12
}

fn default_max_decode_attempts() -> usize {
    24
}

fn default_decode_threads() -> usize {
    std::thread::available_parallelism()
        .map(|threads| threads.get().min(4))
        .unwrap_or(4)
}

// Schema defaults must remain identical across API and executor CPU allocations.
// Runtime defaults still adapt to the CPUs available to the process.
fn schema_default_decode_threads() -> usize {
    4
}

fn schema_default_preprocessing() -> BarcodePreprocessingOptions {
    BarcodePreprocessingOptions {
        decode_threads: schema_default_decode_threads(),
        ..BarcodePreprocessingOptions::default()
    }
}

fn hash_luma(luma: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    luma.hash(&mut hasher);
    hasher.finish()
}

impl Barcode {
    fn from_result(value: RXingResult, variant: &DecodeVariant) -> Self {
        let points: Vec<_> = value
            .getPoints()
            .iter()
            .map(|p| BarcodePoint { x: p.x, y: p.y })
            .map(|p| variant.map_point(p))
            .collect();
        Barcode {
            text: value.getText().to_string(),
            raw_bytes: value.getRawBytes().to_vec(),
            num_bits: value.getNumBits(),
            format: value.getBarcodeFormat().to_string(),
            timestamp: value.getTimestamp(),
            line_count: value.line_count(),
            bounds: Bounds::around(points.iter().map(|point| (point.x, point.y))),
            linear: is_linear(*value.getBarcodeFormat()),
            points,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The format as the options name it (`QR_CODE`, `EAN_13`, …).
    pub fn format_name(&self) -> &str {
        READABLE_BARCODE_FORMATS
            .iter()
            .copied()
            .find(|name| barcode_format_of(name).to_string() == self.format)
            .unwrap_or(self.format.as_str())
    }

    /// `[left, top, right, bottom]` in pixels of the scanned image.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        self.bounds
            .map(|bounds| [bounds.left, bounds.top, bounds.right, bounds.bottom])
    }
}

/// The shortest edge a halved image may have and still be worth scanning.
const MIN_HALVED_EDGE: u32 = 240;

impl DecodeVariant {
    /// The image at half its size, each pixel the mean of four.
    fn halved(&self) -> Option<Self> {
        let (width, height) = (self.width / 2, self.height / 2);
        if width.min(height) < MIN_HALVED_EDGE {
            return None;
        }
        let stride = self.width as usize;
        let mut luma = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height as usize {
            let (upper, _) = self.luma[2 * y * stride..][..2 * width as usize].as_chunks::<2>();
            let (lower, _) =
                self.luma[(2 * y + 1) * stride..][..2 * width as usize].as_chunks::<2>();
            luma.extend(upper.iter().zip(lower).map(|(a, b)| {
                let sum = u16::from(a[0]) + u16::from(a[1]) + u16::from(b[0]) + u16::from(b[1]);
                ((sum + 2) / 4) as u8
            }));
        }
        Some(self.with_luma(luma, width, height, self.point_scale * 2.0))
    }

    /// The image at half its size, a quarter and so on, down to [`MIN_HALVED_EDGE`].
    fn smaller_sizes(&self) -> Vec<Self> {
        let mut sizes = Vec::new();
        let mut halved = self.halved();
        while let Some(image) = halved {
            halved = image.halved();
            sizes.push(image);
        }
        sizes
    }

    fn with_luma(&self, luma: Vec<u8>, width: u32, height: u32, point_scale: f32) -> Self {
        let hash = hash_luma(&luma);
        Self {
            luma,
            width,
            height,
            hash,
            point_scale,
            x_offset: self.x_offset,
            y_offset: self.y_offset,
            base_width: self.base_width,
            base_height: self.base_height,
            rotation: self.rotation,
        }
    }

    fn map_point(&self, point: BarcodePoint) -> BarcodePoint {
        let x = point.x * self.point_scale;
        let y = point.y * self.point_scale;
        let (source_x, source_y) = match self.rotation {
            DecodeRotation::Deg0 => (x, y),
            DecodeRotation::Deg90 => (y, self.base_height.saturating_sub(1) as f32 - x),
            DecodeRotation::Deg180 => (
                self.base_width.saturating_sub(1) as f32 - x,
                self.base_height.saturating_sub(1) as f32 - y,
            ),
            DecodeRotation::Deg270 => (self.base_width.saturating_sub(1) as f32 - y, x),
        };

        BarcodePoint {
            x: source_x + self.x_offset as f32,
            y: source_y + self.y_offset as f32,
        }
    }
}

/// # Detect and Decode (Bar)codes in Images
#[crate::register_node]
#[derive(Default)]
pub struct ReadBarcodesNode {}

impl ReadBarcodesNode {
    pub fn new() -> Self {
        ReadBarcodesNode {}
    }
}

/// rxing's parser knows `rectangular_micro_qr` but not the `_CODE` spelling the options use.
fn barcode_format_of(name: &str) -> BarcodeFormat {
    let name = name.trim();
    if name.eq_ignore_ascii_case("RECTANGULAR_MICRO_QR_CODE") {
        return BarcodeFormat::RECTANGULAR_MICRO_QR_CODE;
    }
    BarcodeFormat::from(name)
}

fn parse_barcode_format(format: &str) -> flow_like_types::Result<BarcodeFormat> {
    let barcode_format = barcode_format_of(format);
    if barcode_format == BarcodeFormat::UNSUPORTED_FORMAT
        || !READABLE_BARCODE_FORMATS
            .iter()
            .any(|supported| barcode_format_of(supported) == barcode_format)
    {
        return Err(anyhow!("Unsupported readable barcode format: {}", format));
    }
    Ok(barcode_format)
}

fn readable_barcode_formats() -> Vec<BarcodeFormat> {
    READABLE_BARCODE_FORMATS
        .iter()
        .map(|format| barcode_format_of(format))
        .filter(|format| *format != BarcodeFormat::UNSUPORTED_FORMAT)
        .collect()
}

fn push_unique_format(formats: &mut Vec<BarcodeFormat>, format: BarcodeFormat) {
    if !formats.contains(&format) {
        formats.push(format);
    }
}

fn parse_preprocess_mode(mode: &str) -> flow_like_types::Result<PreprocessMode> {
    match mode.trim().to_lowercase().as_str() {
        "none" => Ok(PreprocessMode::None),
        "balanced" => Ok(PreprocessMode::Balanced),
        "fallback" => Ok(PreprocessMode::Fallback),
        "aggressive" => Ok(PreprocessMode::Aggressive),
        "industrial" => Ok(PreprocessMode::Industrial),
        _ => Err(anyhow!("Unsupported barcode preprocessing mode: {}", mode)),
    }
}

fn parse_polarity(polarity: &str) -> flow_like_types::Result<BarcodePolarity> {
    match polarity.trim().to_lowercase().as_str() {
        "auto" => Ok(BarcodePolarity::Auto),
        "darkonlight" | "dark_on_light" | "dark-on-light" => Ok(BarcodePolarity::DarkOnLight),
        "lightondark" | "light_on_dark" | "light-on-dark" => Ok(BarcodePolarity::LightOnDark),
        _ => Err(anyhow!("Unsupported barcode polarity: {}", polarity)),
    }
}

fn parse_rotation(degrees: u16) -> flow_like_types::Result<DecodeRotation> {
    match degrees % 360 {
        0 => Ok(DecodeRotation::Deg0),
        90 => Ok(DecodeRotation::Deg90),
        180 => Ok(DecodeRotation::Deg180),
        270 => Ok(DecodeRotation::Deg270),
        _ => Err(anyhow!(
            "Unsupported barcode rotation: {}. Use 0, 90, 180, or 270",
            degrees
        )),
    }
}

fn effective_rotations(
    preprocess_mode: PreprocessMode,
    preprocessing: &BarcodePreprocessingOptions,
) -> flow_like_types::Result<Vec<DecodeRotation>> {
    let rotation_degrees = if preprocessing.rotations.is_empty() {
        if preprocess_mode == PreprocessMode::Industrial {
            vec![0, 90, 180, 270]
        } else {
            vec![0, 90]
        }
    } else {
        preprocessing.rotations.clone()
    };

    let mut rotations = Vec::new();
    for degrees in rotation_degrees {
        let rotation = parse_rotation(degrees)?;
        if !rotations.contains(&rotation) {
            rotations.push(rotation);
        }
    }

    Ok(rotations)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScanArea {
    Whole,
    /// The four quarters and the middle. Aztec and MaxiCode are only found around the centre
    /// of what is scanned, and a tile gives the other matrix codes a second threshold.
    Tiles,
}

fn scan_areas(area: ScanArea, width: u32, height: u32) -> Vec<(u32, u32, u32, u32)> {
    let (half_width, half_height) = (width / 2, height / 2);
    match area {
        ScanArea::Whole => vec![(0, 0, width, height)],
        ScanArea::Tiles => vec![
            (0, 0, half_width, half_height),
            (half_width, 0, width - half_width, half_height),
            (0, half_height, half_width, height - half_height),
            (
                half_width,
                half_height,
                width - half_width,
                height - half_height,
            ),
            (width / 4, height / 4, half_width, half_height),
        ],
    }
}

/// The symbols of `formats` in one variant, in pixels of the source image.
fn scan_variant(
    variant: &DecodeVariant,
    hints: &DecodeHints,
    formats: &[BarcodeFormat],
    area: ScanArea,
) -> Vec<Barcode> {
    let matrix: Vec<_> = formats
        .iter()
        .copied()
        .filter(|format| !is_linear(*format))
        .collect();
    // Rows cross the whole image already; a tile border would only cut linear codes apart.
    let formats = match area {
        ScanArea::Whole => formats,
        ScanArea::Tiles => matrix.as_slice(),
    };
    // A matrix code that fills the photo has modules wider than the readers' fixed windows.
    // Halved, they fit again and the sensor's noise averages out. Bars read no better there.
    let smaller = match area {
        ScanArea::Whole if !matrix.is_empty() => variant.smaller_sizes(),
        _ => Vec::new(),
    };
    let sizes: Vec<_> = std::iter::once((variant, formats))
        .chain(smaller.iter().map(|image| (image, matrix.as_slice())))
        .filter(|(_, formats)| !formats.is_empty())
        .collect();

    let barcodes = scan_sizes(&sizes, hints, area, Polarity::AsIs);
    // Light codes on dark are looked for where the image shows no dark ones on light.
    if barcodes.is_empty() && hints.AlsoInverted == Some(true) {
        return scan_sizes(&sizes, hints, area, Polarity::Inverted);
    }
    barcodes
}

/// The symbols of one polarity in a variant at each of its sizes, each size with its formats.
fn scan_sizes(
    sizes: &[(&DecodeVariant, &[BarcodeFormat])],
    hints: &DecodeHints,
    area: ScanArea,
    polarity: Polarity,
) -> Vec<Barcode> {
    let mut barcodes = Vec::new();
    for (image, formats) in sizes {
        let mut scanner = Scanner::new(hints, formats);
        for area in scan_areas(area, image.width, image.height) {
            for symbol in scan_area(&mut scanner, image, area, polarity) {
                merge_barcode(&mut barcodes, Barcode::from_result(symbol, image));
            }
        }
    }
    barcodes
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Polarity {
    AsIs,
    /// rxing's AlsoInverted flips only its 2D matrix. Inverted pixels cover 1D rows too.
    Inverted,
}

/// The symbols in one `(left, top, width, height)` area of a variant, in the variant's pixels.
fn scan_area(
    scanner: &mut Scanner,
    variant: &DecodeVariant,
    area: (u32, u32, u32, u32),
    polarity: Polarity,
) -> Vec<RXingResult> {
    let (left, top, width, height) = area;
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let (mut pixels, _, _, _, _) = crop_luma_to_roi(
        &variant.luma,
        variant.width,
        variant.height,
        Some(&BarcodeRoi {
            x: left,
            y: top,
            width,
            height,
        }),
    );
    if pixels.iter().all(|pixel| *pixel == pixels[0]) {
        return Vec::new();
    }
    if polarity == Polarity::Inverted {
        pixels = invert_luma(&pixels);
    }
    let mut symbols = scanner.scan(&pixels, width, height);
    for symbol in &mut symbols {
        for point in symbol.getPointsMut() {
            point.x += left as f32;
            point.y += top as f32;
        }
    }
    symbols
}

/// The codes in an 8-bit grayscale image (`width * height` bytes), decoded as the Read Barcodes
/// node does with `options`.
pub fn read_barcodes_in_luma(
    luma: Vec<u8>,
    width: u32,
    height: u32,
    options: &ReadBarcodeOptions,
) -> flow_like_types::Result<Vec<Barcode>> {
    if options.max_results == Some(0) {
        return Err(anyhow!(
            "max_results must be greater than zero when provided"
        ));
    }
    let mode = parse_preprocess_mode(&options.preprocess)?;
    let preprocessing = effective_preprocessing_options(mode, &options.preprocessing);
    let polarity = parse_polarity(&preprocessing.polarity)?;
    let hints = DecodeHints {
        TryHarder: Some(options.try_harder),
        AlsoInverted: Some(polarity == BarcodePolarity::Auto && options.also_inverted),
        PureBarcode: Some(options.pure_barcode),
        ..DecodeHints::default()
    };
    let formats = scan_formats(options)?;
    let turns = Turns::plan(mode, &preprocessing, &formats)?;
    let mut variants = build_decode_variants(
        luma,
        width,
        height,
        PreprocessMode::None,
        polarity,
        &turns.rotations,
        &preprocessing,
    );
    let attempts = DecodeAttempts {
        hints: &hints,
        formats: &formats,
        rotated_formats: turns.formats.as_deref(),
        decode_threads: preprocessing.decode_threads,
        max_results: options.max_results,
        validation: &options.validation,
    };
    Ok(attempts.read(&mut variants, mode, &preprocessing))
}

/// The rotations the image is scanned in.
struct Turns {
    rotations: Vec<DecodeRotation>,
    /// What the turned copies are scanned for; everything when the options name the rotations.
    formats: Option<Vec<BarcodeFormat>>,
}

impl Turns {
    /// Rotations nobody asked for are there for linear codes, which only read along their
    /// bars. Matrix readers find a turned symbol themselves, so they scan the upright image.
    fn plan(
        mode: PreprocessMode,
        preprocessing: &BarcodePreprocessingOptions,
        formats: &[BarcodeFormat],
    ) -> flow_like_types::Result<Self> {
        if !preprocessing.rotations.is_empty() || mode == PreprocessMode::Industrial {
            return Ok(Self {
                rotations: effective_rotations(mode, preprocessing)?,
                formats: None,
            });
        }
        let linear: Vec<_> = formats
            .iter()
            .copied()
            .filter(|format| is_linear(*format))
            .collect();
        let rotations = if linear.is_empty() {
            vec![DecodeRotation::Deg0]
        } else {
            effective_rotations(mode, preprocessing)?
        };
        Ok(Self {
            rotations,
            formats: Some(linear),
        })
    }
}

/// The formats `options` ask for; every readable one when they name none.
fn scan_formats(options: &ReadBarcodeOptions) -> flow_like_types::Result<Vec<BarcodeFormat>> {
    let mut formats = Vec::new();
    for format in &options.expected_formats {
        push_unique_format(&mut formats, parse_barcode_format(format)?);
    }
    if formats.is_empty() && options.filter {
        formats.push(parse_barcode_format(&options.format)?);
    }
    if formats.is_empty() {
        formats = readable_barcode_formats();
    }
    Ok(formats)
}

struct DecodeAttempts<'a> {
    hints: &'a DecodeHints,
    formats: &'a [BarcodeFormat],
    /// What rotated variants are scanned for, when the rotation was not asked for.
    rotated_formats: Option<&'a [BarcodeFormat]>,
    decode_threads: usize,
    max_results: Option<usize>,
    validation: &'a BarcodeValidationOptions,
}

impl DecodeAttempts<'_> {
    /// Scans the variants as they are, then the preprocessed ones the mode adds, then tiles,
    /// until `max_decode_attempts` scans are spent or the result limit is reached.
    fn read(
        &self,
        variants: &mut Vec<DecodeVariant>,
        mode: PreprocessMode,
        preprocessing: &BarcodePreprocessingOptions,
    ) -> Vec<Barcode> {
        let mut budget = preprocessing.max_decode_attempts.max(1);
        let mut barcodes = Vec::new();
        // A result limit or Fallback mode may end the search after the unprocessed variants,
        // and then the others are never built. Otherwise all of them scan side by side.
        let staged = matches!(mode, PreprocessMode::None | PreprocessMode::Fallback)
            || self.max_results.is_some();
        let mut scanned = 0;
        if staged {
            scanned = variants.len().min(budget);
            self.run(&variants[..scanned], ScanArea::Whole, &mut barcodes);
            budget -= scanned;
            if budget == 0 || self.unprocessed_suffice(mode, &barcodes) {
                return barcodes;
            }
        }
        append_preprocessed_variants(variants, mode, preprocessing);
        let whole = &variants[scanned..];
        let whole = &whole[..whole.len().min(budget)];
        self.run(whole, ScanArea::Whole, &mut barcodes);
        budget -= whole.len();

        // Tiles recover images where no symbol could be located; thorough modes
        // also search them after successful whole-image passes.
        let thorough = matches!(
            mode,
            PreprocessMode::Aggressive | PreprocessMode::Industrial
        );
        if barcodes.is_empty() || thorough {
            let tiled = &variants[..variants.len().min(budget)];
            self.run(tiled, ScanArea::Tiles, &mut barcodes);
        }
        barcodes
    }

    fn limit_reached(&self, barcodes: &[Barcode]) -> bool {
        self.max_results
            .is_some_and(|limit| barcodes.len() >= limit)
    }

    /// The search ends with what the unprocessed variants gave.
    fn unprocessed_suffice(&self, mode: PreprocessMode, barcodes: &[Barcode]) -> bool {
        match mode {
            PreprocessMode::None => true,
            PreprocessMode::Fallback if !barcodes.is_empty() => true,
            _ => self.limit_reached(barcodes),
        }
    }

    fn threads(&self) -> usize {
        std::thread::available_parallelism()
            .map(|threads| threads.get().min(self.decode_threads.max(1)))
            .unwrap_or(1)
            .min(self.max_results.unwrap_or(usize::MAX))
            .max(1)
    }

    fn scan(&self, variant: &DecodeVariant, area: ScanArea) -> Vec<Barcode> {
        let formats = if variant.rotation == DecodeRotation::Deg0 {
            self.formats
        } else {
            self.rotated_formats.unwrap_or(self.formats)
        };
        scan_variant(variant, self.hints, formats, area)
    }

    fn scan_together(&self, variants: &[DecodeVariant], area: ScanArea) -> Vec<Vec<Barcode>> {
        // Rayon's shared pool bounds workers across concurrent node executions.
        #[cfg(feature = "execute")]
        if variants.len() > 1 {
            return variants
                .par_iter()
                .map(|variant| self.scan(variant, area))
                .collect();
        }
        variants
            .iter()
            .map(|variant| self.scan(variant, area))
            .collect()
    }

    fn run(&self, variants: &[DecodeVariant], area: ScanArea, barcodes: &mut Vec<Barcode>) {
        for together in variants.chunks(self.threads()) {
            if self.limit_reached(barcodes) {
                break;
            }
            for barcode in self.scan_together(together, area).into_iter().flatten() {
                if barcode_matches_validation(&barcode, self.validation) {
                    merge_barcode(barcodes, barcode);
                }
            }
        }
        if let Some(limit) = self.max_results {
            barcodes.truncate(limit);
        }
    }
}

fn append_processed_variants<F>(
    variants: &mut Vec<DecodeVariant>,
    source_count: usize,
    max_variants: usize,
    build_variant: F,
) where
    F: FnMut(&DecodeVariant) -> Option<DecodeVariant>,
{
    if variants.len() >= max_variants || source_count == 0 {
        return;
    }

    let source_count = source_count.min(variants.len());
    let generated: Vec<DecodeVariant> = variants[..source_count]
        .iter()
        .filter_map(build_variant)
        .take(max_variants.saturating_sub(variants.len()))
        .collect();

    for variant in generated {
        push_decode_variant(variants, variant, max_variants);
    }
}

fn build_decode_variants(
    luma: Vec<u8>,
    width: u32,
    height: u32,
    preprocess_mode: PreprocessMode,
    polarity: BarcodePolarity,
    rotations: &[DecodeRotation],
    preprocessing: &BarcodePreprocessingOptions,
) -> Vec<DecodeVariant> {
    let mut variants = Vec::new();
    let max_variants = preprocessing.max_variants.max(1);
    let (mut source_luma, source_width, source_height, x_offset, y_offset) =
        crop_luma_to_roi(&luma, width, height, preprocessing.roi.as_ref());

    if polarity == BarcodePolarity::LightOnDark {
        source_luma = invert_luma(&source_luma);
    }

    for rotation in rotations {
        let (rotated_luma, rotated_width, rotated_height) =
            rotate_luma(&source_luma, source_width, source_height, *rotation);
        push_decode_variant(
            &mut variants,
            DecodeVariant {
                hash: hash_luma(&rotated_luma),
                luma: rotated_luma,
                width: rotated_width,
                height: rotated_height,
                point_scale: 1.0,
                x_offset,
                y_offset,
                base_width: source_width,
                base_height: source_height,
                rotation: *rotation,
            },
            max_variants,
        );
    }

    if preprocess_mode == PreprocessMode::None {
        return variants;
    }

    append_preprocessed_variants(&mut variants, preprocess_mode, preprocessing);
    variants
}

fn append_preprocessed_variants(
    variants: &mut Vec<DecodeVariant>,
    preprocess_mode: PreprocessMode,
    preprocessing: &BarcodePreprocessingOptions,
) {
    let Some(source) = variants.first() else {
        return;
    };
    let (source_width, source_height) = (source.base_width, source.base_height);
    let max_variants = preprocessing.max_variants.max(1);
    if preprocessing.contrast_stretch {
        let source_count = variants.len();
        append_processed_variants(variants, source_count, max_variants, |variant| {
            Some(variant.with_luma(
                contrast_stretch_luma(&variant.luma),
                variant.width,
                variant.height,
                variant.point_scale,
            ))
        });
    }

    if preprocessing.local_contrast {
        let source_count = variants.len();
        append_processed_variants(variants, source_count, max_variants, |variant| {
            Some(variant.with_luma(
                local_contrast_stretch_luma(
                    &variant.luma,
                    variant.width,
                    variant.height,
                    preprocessing.local_contrast_tile_size,
                ),
                variant.width,
                variant.height,
                variant.point_scale,
            ))
        });
    }

    let base_variant_count = variants.len();
    if preprocessing.denoise {
        append_processed_variants(variants, base_variant_count, max_variants, |variant| {
            Some(variant.with_luma(
                median_filter_3x3_luma(&variant.luma, variant.width, variant.height),
                variant.width,
                variant.height,
                variant.point_scale,
            ))
        });
    }

    let enhanced_variant_count = variants.len();
    if preprocessing.sharpen || preprocess_mode == PreprocessMode::Industrial {
        append_processed_variants(variants, enhanced_variant_count, max_variants, |variant| {
            Some(variant.with_luma(
                sharpen_luma(&variant.luma, variant.width, variant.height),
                variant.width,
                variant.height,
                variant.point_scale,
            ))
        });
    }

    let threshold_source_count = variants.len();
    if preprocessing.otsu_threshold {
        append_processed_variants(variants, threshold_source_count, max_variants, |variant| {
            Some(variant.with_luma(
                otsu_threshold_luma(&variant.luma),
                variant.width,
                variant.height,
                variant.point_scale,
            ))
        });
    }

    if preprocessing.adaptive_threshold {
        append_processed_variants(variants, threshold_source_count, max_variants, |variant| {
            Some(variant.with_luma(
                adaptive_threshold_luma(
                    &variant.luma,
                    variant.width,
                    variant.height,
                    preprocessing.adaptive_threshold_window,
                    preprocessing.adaptive_threshold_bias,
                ),
                variant.width,
                variant.height,
                variant.point_scale,
            ))
        });
    }

    if preprocessing.morphology {
        let morphology_source_count = variants.len();
        let mut generated = Vec::new();
        for variant in variants[..morphology_source_count].iter() {
            if generated.len() >= max_variants.saturating_sub(variants.len()) {
                break;
            }
            if !is_binary_luma(&variant.luma) {
                continue;
            }

            let opened = open_dark_binary_luma(&variant.luma, variant.width, variant.height);
            generated.push(variant.with_luma(
                opened.clone(),
                variant.width,
                variant.height,
                variant.point_scale,
            ));

            if generated.len() >= max_variants.saturating_sub(variants.len()) {
                break;
            }
            generated.push(variant.with_luma(
                close_dark_binary_luma(&variant.luma, variant.width, variant.height),
                variant.width,
                variant.height,
                variant.point_scale,
            ));

            if generated.len() >= max_variants.saturating_sub(variants.len()) {
                break;
            }
            generated.push(variant.with_luma(
                close_dark_binary_luma(&opened, variant.width, variant.height),
                variant.width,
                variant.height,
                variant.point_scale,
            ));
        }

        for variant in generated {
            push_decode_variant(variants, variant, max_variants);
        }
    }

    let upscale_factor = preprocessing.upscale_factor.clamp(1, 4);
    if upscale_factor > 1
        && should_upscale(
            source_width,
            source_height,
            upscale_factor,
            preprocessing.max_upscale_pixels,
        )
    {
        let upscale_source_count = if preprocess_mode == PreprocessMode::Fallback {
            variants.len().min(1)
        } else {
            variants.len()
        };
        append_processed_variants(variants, upscale_source_count, max_variants, |variant| {
            let (upscaled, upscaled_width, upscaled_height) =
                upscale_luma_nearest(&variant.luma, variant.width, variant.height, upscale_factor);
            Some(variant.with_luma(
                upscaled,
                upscaled_width,
                upscaled_height,
                variant.point_scale / upscale_factor as f32,
            ))
        });
    }
}

fn push_decode_variant(
    variants: &mut Vec<DecodeVariant>,
    variant: DecodeVariant,
    max_variants: usize,
) {
    if variants.len() >= max_variants {
        return;
    }

    if variants.iter().any(|existing| {
        existing.width == variant.width
            && existing.height == variant.height
            && existing.hash == variant.hash
    }) {
        return;
    }

    variants.push(variant);
}

fn crop_luma_to_roi(
    luma: &[u8],
    width: u32,
    height: u32,
    roi: Option<&BarcodeRoi>,
) -> (Vec<u8>, u32, u32, u32, u32) {
    let Some(roi) = roi else {
        return (luma.to_vec(), width, height, 0, 0);
    };

    if roi.width == 0 || roi.height == 0 || roi.x >= width || roi.y >= height {
        return (luma.to_vec(), width, height, 0, 0);
    }

    let crop_width = roi.width.min(width - roi.x);
    let crop_height = roi.height.min(height - roi.y);
    let mut cropped = vec![0u8; (crop_width * crop_height) as usize];

    for y in 0..crop_height {
        let source_offset = ((roi.y + y) * width + roi.x) as usize;
        let target_offset = (y * crop_width) as usize;
        cropped[target_offset..target_offset + crop_width as usize]
            .copy_from_slice(&luma[source_offset..source_offset + crop_width as usize]);
    }

    (cropped, crop_width, crop_height, roi.x, roi.y)
}

fn rotate_luma(
    luma: &[u8],
    width: u32,
    height: u32,
    rotation: DecodeRotation,
) -> (Vec<u8>, u32, u32) {
    match rotation {
        DecodeRotation::Deg0 => (luma.to_vec(), width, height),
        DecodeRotation::Deg90 => {
            let mut rotated = vec![0u8; luma.len()];
            for y in 0..height {
                for x in 0..width {
                    let target_x = height - 1 - y;
                    let target_y = x;
                    rotated[(target_y * height + target_x) as usize] =
                        luma[(y * width + x) as usize];
                }
            }
            (rotated, height, width)
        }
        DecodeRotation::Deg180 => {
            let mut rotated = vec![0u8; luma.len()];
            for y in 0..height {
                for x in 0..width {
                    let target_x = width - 1 - x;
                    let target_y = height - 1 - y;
                    rotated[(target_y * width + target_x) as usize] =
                        luma[(y * width + x) as usize];
                }
            }
            (rotated, width, height)
        }
        DecodeRotation::Deg270 => {
            let mut rotated = vec![0u8; luma.len()];
            for y in 0..height {
                for x in 0..width {
                    let target_x = y;
                    let target_y = width - 1 - x;
                    rotated[(target_y * height + target_x) as usize] =
                        luma[(y * width + x) as usize];
                }
            }
            (rotated, height, width)
        }
    }
}

fn invert_luma(luma: &[u8]) -> Vec<u8> {
    luma.iter().map(|pixel| 255 - *pixel).collect()
}

fn barcode_matches_validation(barcode: &Barcode, validation: &BarcodeValidationOptions) -> bool {
    text_matches_validation(&barcode.text, validation)
}

fn text_matches_validation(text: &str, validation: &BarcodeValidationOptions) -> bool {
    if let Some(min_text_length) = validation.min_text_length
        && text.len() < min_text_length
    {
        return false;
    }

    if let Some(max_text_length) = validation.max_text_length
        && text.len() > max_text_length
    {
        return false;
    }

    if let Some(prefix) = &validation.text_prefix
        && !text.starts_with(prefix)
    {
        return false;
    }

    if let Some(suffix) = &validation.text_suffix
        && !text.ends_with(suffix)
    {
        return false;
    }

    if let Some(contains) = &validation.text_contains
        && !text.contains(contains)
    {
        return false;
    }

    if !validation.allowed_texts.is_empty()
        && !validation
            .allowed_texts
            .iter()
            .any(|allowed| allowed == text)
    {
        return false;
    }

    true
}

fn contrast_stretch_luma(luma: &[u8]) -> Vec<u8> {
    let Some(min) = luma.iter().min().copied() else {
        return Vec::new();
    };
    let Some(max) = luma.iter().max().copied() else {
        return Vec::new();
    };
    if min == max {
        return luma.to_vec();
    }

    let range = u16::from(max - min);
    luma.iter()
        .map(|pixel| (((u16::from(*pixel - min)) * 255) / range) as u8)
        .collect()
}

fn local_contrast_stretch_luma(luma: &[u8], width: u32, height: u32, tile_size: u32) -> Vec<u8> {
    let tile_size = tile_size.max(8);
    let mut output = luma.to_vec();

    for tile_y in (0..height).step_by(tile_size as usize) {
        let tile_height = tile_size.min(height - tile_y);
        for tile_x in (0..width).step_by(tile_size as usize) {
            let tile_width = tile_size.min(width - tile_x);
            let mut min = u8::MAX;
            let mut max = u8::MIN;

            for y in tile_y..tile_y + tile_height {
                let row_offset = (y * width) as usize;
                for x in tile_x..tile_x + tile_width {
                    let pixel = luma[row_offset + x as usize];
                    min = min.min(pixel);
                    max = max.max(pixel);
                }
            }

            if min == max {
                continue;
            }

            let range = u16::from(max - min);
            for y in tile_y..tile_y + tile_height {
                let row_offset = (y * width) as usize;
                for x in tile_x..tile_x + tile_width {
                    let index = row_offset + x as usize;
                    output[index] = (((u16::from(luma[index] - min)) * 255) / range) as u8;
                }
            }
        }
    }

    output
}

fn median_filter_3x3_luma(luma: &[u8], width: u32, height: u32) -> Vec<u8> {
    if width < 3 || height < 3 {
        return luma.to_vec();
    }

    let mut output = luma.to_vec();
    let mut window = [0u8; 9];

    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let mut i = 0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let source_x = (x as i32 + dx) as u32;
                    let source_y = (y as i32 + dy) as u32;
                    window[i] = luma[(source_y * width + source_x) as usize];
                    i += 1;
                }
            }

            window.sort_unstable();
            output[(y * width + x) as usize] = window[4];
        }
    }

    output
}

/// Half the width of the box the unsharp mask blurs with. Two passes of it reach about as far
/// as the modules of a code a camera can still resolve.
const SHARPEN_RADIUS: usize = 2;
/// How many times the detail the blur removed is added back. More reads fewer codes: the
/// sensor's noise is sharpened along with the modules.
const SHARPEN_AMOUNT: i32 = 1;

/// Sums of `2 * radius + 1` neighbours along each row, the edge pixels repeated.
fn box_sum_rows(source: &[u32], target: &mut [u32], width: usize, radius: usize) {
    let reach = radius as isize;
    for (row, sums) in source
        .chunks_exact(width)
        .zip(target.chunks_exact_mut(width))
    {
        let at = |index: isize| row[index.clamp(0, width as isize - 1) as usize];
        let mut sum: u32 = (-reach..=reach).map(at).sum();
        for (x, out) in sums.iter_mut().enumerate() {
            *out = sum;
            sum = sum + at(x as isize + reach + 1) - at(x as isize - reach);
        }
    }
}

/// Sums of `2 * radius + 1` neighbours along each column, the edge pixels repeated.
fn box_sum_columns(source: &[u32], target: &mut [u32], width: usize, radius: usize) {
    let height = source.len() / width;
    let reach = radius as isize;
    let row = |index: isize| {
        let y = index.clamp(0, height as isize - 1) as usize;
        &source[y * width..(y + 1) * width]
    };
    let mut sums = vec![0u32; width];
    for index in -reach..=reach {
        for (sum, value) in sums.iter_mut().zip(row(index)) {
            *sum += value;
        }
    }
    for (y, out) in target.chunks_exact_mut(width).enumerate() {
        out.copy_from_slice(&sums);
        let entering = row(y as isize + reach + 1);
        let leaving = row(y as isize - reach);
        for ((sum, enter), leave) in sums.iter_mut().zip(entering).zip(leaving) {
            *sum = *sum + enter - leave;
        }
    }
}

/// An unsharp mask. A camera that is slightly out of focus, or smoothing a dim frame, flattens
/// the modules of a code towards grey; this pulls them apart again before thresholding.
fn sharpen_luma(luma: &[u8], width: u32, height: u32) -> Vec<u8> {
    if width < 3 || height < 3 {
        return luma.to_vec();
    }

    let width = width as usize;
    let mut blurred: Vec<u32> = luma.iter().map(|pixel| u32::from(*pixel)).collect();
    let mut scratch = vec![0u32; blurred.len()];
    box_sum_rows(&blurred, &mut scratch, width, SHARPEN_RADIUS);
    box_sum_rows(&scratch, &mut blurred, width, SHARPEN_RADIUS);
    box_sum_columns(&blurred, &mut scratch, width, SHARPEN_RADIUS);
    box_sum_columns(&scratch, &mut blurred, width, SHARPEN_RADIUS);
    let scale = (2 * SHARPEN_RADIUS as i32 + 1).pow(4);
    luma.iter()
        .zip(&blurred)
        .map(|(pixel, blurred)| {
            let pixel = i32::from(*pixel) * scale;
            ((pixel + SHARPEN_AMOUNT * (pixel - *blurred as i32)) / scale).clamp(0, 255) as u8
        })
        .collect()
}

fn otsu_threshold_luma(luma: &[u8]) -> Vec<u8> {
    let threshold = otsu_threshold(luma);
    luma.iter()
        .map(|pixel| if *pixel <= threshold { 0 } else { 255 })
        .collect()
}

fn otsu_threshold(luma: &[u8]) -> u8 {
    if luma.is_empty() {
        return 127;
    }

    let mut histogram = [0u32; 256];
    for pixel in luma {
        histogram[*pixel as usize] += 1;
    }

    let total = luma.len() as f64;
    let sum_total = histogram
        .iter()
        .enumerate()
        .map(|(value, count)| (value as f64) * (*count as f64))
        .sum::<f64>();

    let mut sum_background = 0.0;
    let mut weight_background = 0.0;
    let mut max_variance = 0.0;
    let mut threshold = 127u8;

    for (value, count) in histogram.iter().enumerate() {
        weight_background += *count as f64;
        if weight_background == 0.0 {
            continue;
        }

        let weight_foreground = total - weight_background;
        if weight_foreground == 0.0 {
            break;
        }

        sum_background += (value as f64) * (*count as f64);
        let mean_background = sum_background / weight_background;
        let mean_foreground = (sum_total - sum_background) / weight_foreground;
        let variance = weight_background
            * weight_foreground
            * (mean_background - mean_foreground)
            * (mean_background - mean_foreground);

        if variance > max_variance {
            max_variance = variance;
            threshold = value as u8;
        }
    }

    threshold
}

fn adaptive_threshold_luma(
    luma: &[u8],
    width: u32,
    height: u32,
    window_size: u32,
    bias: i16,
) -> Vec<u8> {
    if width == 0 || height == 0 || luma.is_empty() {
        return Vec::new();
    }

    let radius = (window_size.max(3) | 1) / 2;
    let width_usize = width as usize;
    let integral_width = width_usize + 1;
    let mut integral = vec![0u64; integral_width * (height as usize + 1)];

    for y in 0..height as usize {
        let mut row_sum = 0u64;
        for x in 0..width_usize {
            row_sum += u64::from(luma[y * width_usize + x]);
            let above = integral[y * integral_width + x + 1];
            integral[(y + 1) * integral_width + x + 1] = above + row_sum;
        }
    }

    let mut output = vec![0u8; luma.len()];
    for y in 0..height {
        let y0 = y.saturating_sub(radius) as usize;
        let y1 = (y + radius + 1).min(height) as usize;
        for x in 0..width {
            let x0 = x.saturating_sub(radius) as usize;
            let x1 = (x + radius + 1).min(width) as usize;
            let area = ((x1 - x0) * (y1 - y0)) as u64;
            let bottom_right = integral[y1 * integral_width + x1] as i128;
            let top_right = integral[y0 * integral_width + x1] as i128;
            let bottom_left = integral[y1 * integral_width + x0] as i128;
            let top_left = integral[y0 * integral_width + x0] as i128;
            let sum = (bottom_right + top_left - top_right - bottom_left).max(0) as u64;
            let mean = (sum / area) as i16;
            let threshold = (mean - bias).clamp(0, 255) as u8;
            let index = (y * width + x) as usize;
            output[index] = if luma[index] <= threshold { 0 } else { 255 };
        }
    }

    output
}

fn is_binary_luma(luma: &[u8]) -> bool {
    luma.iter().all(|pixel| *pixel == 0 || *pixel == 255)
}

fn open_dark_binary_luma(luma: &[u8], width: u32, height: u32) -> Vec<u8> {
    min_filter_3x3_luma(&max_filter_3x3_luma(luma, width, height), width, height)
}

fn close_dark_binary_luma(luma: &[u8], width: u32, height: u32) -> Vec<u8> {
    max_filter_3x3_luma(&min_filter_3x3_luma(luma, width, height), width, height)
}

fn min_filter_3x3_luma(luma: &[u8], width: u32, height: u32) -> Vec<u8> {
    if width < 3 || height < 3 {
        return luma.to_vec();
    }

    let mut output = luma.to_vec();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let mut min = u8::MAX;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let source_x = (x as i32 + dx) as u32;
                    let source_y = (y as i32 + dy) as u32;
                    min = min.min(luma[(source_y * width + source_x) as usize]);
                }
            }
            output[(y * width + x) as usize] = min;
        }
    }

    output
}

fn max_filter_3x3_luma(luma: &[u8], width: u32, height: u32) -> Vec<u8> {
    if width < 3 || height < 3 {
        return luma.to_vec();
    }

    let mut output = luma.to_vec();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let mut max = u8::MIN;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let source_x = (x as i32 + dx) as u32;
                    let source_y = (y as i32 + dy) as u32;
                    max = max.max(luma[(source_y * width + source_x) as usize]);
                }
            }
            output[(y * width + x) as usize] = max;
        }
    }

    output
}

fn should_upscale(width: u32, height: u32, factor: u32, max_upscale_pixels: u64) -> bool {
    u64::from(width) * u64::from(height) * u64::from(factor) * u64::from(factor)
        <= max_upscale_pixels
}

fn upscale_luma_nearest(luma: &[u8], width: u32, height: u32, factor: u32) -> (Vec<u8>, u32, u32) {
    let upscaled_width = width * factor;
    let upscaled_height = height * factor;
    let mut upscaled = vec![0u8; (upscaled_width * upscaled_height) as usize];

    for y in 0..height {
        for x in 0..width {
            let pixel = luma[(y * width + x) as usize];
            for dy in 0..factor {
                for dx in 0..factor {
                    let target_x = x * factor + dx;
                    let target_y = y * factor + dy;
                    upscaled[(target_y * upscaled_width + target_x) as usize] = pixel;
                }
            }
        }
    }

    (upscaled, upscaled_width, upscaled_height)
}

/// Adds `candidate` unless the list already holds what is printed at its place. Variants and
/// tiles read one symbol many times, and now and then one of them reads it differently.
fn merge_barcode(barcodes: &mut Vec<Barcode>, candidate: Barcode) {
    let same_place = |existing: &Barcode| match (existing.bounds, candidate.bounds) {
        (Some(a), Some(b)) => a.same_place(&b),
        // Without a position there is only the content to compare.
        _ => existing.text == candidate.text && existing.format == candidate.format,
    };
    let Some(existing) = barcodes.iter_mut().find(|existing| same_place(existing)) else {
        barcodes.push(candidate);
        return;
    };
    if existing.text == candidate.text && existing.format == candidate.format {
        existing.line_count = existing.line_count.max(candidate.line_count);
        return;
    }
    match (existing.linear, candidate.linear) {
        // Two readings of one run of bars: the one more rows agree on, the longer on a tie.
        (true, true) => {
            if (candidate.line_count, candidate.text.len())
                > (existing.line_count, existing.text.len())
            {
                *existing = candidate;
            }
        }
        // Rows through a matrix code sometimes pass for a short linear one.
        (true, false) => *existing = candidate,
        (false, true) => {}
        (false, false) => barcodes.push(candidate),
    }
}

#[async_trait]
impl NodeLogic for ReadBarcodesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "read_barcodes",
            "Read QR-/Barcode",
            "Read/Decode QR Codes and Barcodes",
            "Image/Content",
        );
        node.set_flowscript_name("image", "readBarcodes");
        node.set_receiver("image_in");
        node.set_version(7);
        node.add_icon("/flow/icons/barcode.svg");

        // inputs
        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("image_in", "Image", "Image object", VariableType::Struct)
            .set_schema::<NodeImage>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "options",
            "Options",
            "Barcode decoding options",
            VariableType::Struct,
        )
        .set_schema::<ReadBarcodeOptions>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(ReadBarcodeOptions::default())));

        // outputs
        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "results",
            "Results",
            "Detected/Decoded Codes",
            VariableType::Struct,
        )
        .set_schema::<Barcode>()
        .set_value_type(flow_like::flow::pin::ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let options: ReadBarcodeOptions = context.evaluate_pin("options").await?;
        let node_img: NodeImage = context.evaluate_pin("image_in").await?;
        let img = node_img.get_image(context).await?;

        let results = tokio::task::spawn_blocking(move || {
            let gray = img.blocking_lock().to_luma8();
            let (w, h) = gray.dimensions();
            read_barcodes_in_luma(gray.into_raw(), w, h, &options)
        })
        .await
        .map_err(|error| anyhow!("Barcode decode task failed: {}", error))??;
        if results.is_empty() {
            context.log_message(
                "No codes matched the configured formats and validation constraints.",
                LogLevel::Warn,
            );
        }

        // set outputs
        context.set_pin_value("results", json!(results)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::image::{
        DynamicImage, GenericImageView, GrayImage, ImageBuffer, Luma,
        imageops::{FilterType, overlay},
    };
    use imageproc::geometric_transformations::{Interpolation, rotate_about_center};
    use rxing::{EncodeHints, MultiFormatWriter, Writer};

    fn resolve_schema<'a>(
        root: &'a flow_like_types::Value,
        schema: &'a flow_like_types::Value,
    ) -> &'a flow_like_types::Value {
        match schema.get("$ref").and_then(|value| value.as_str()) {
            Some(reference) => root
                .pointer(reference.strip_prefix('#').expect("local schema reference"))
                .expect("referenced schema definition"),
            None => schema,
        }
    }

    #[test]
    fn barcode_schema_thread_defaults_are_independent_of_cpu_allocation() {
        let schema = flow_like_types::json::to_value(schemars::schema_for!(ReadBarcodeOptions))
            .expect("schema should serialize");
        let preprocessing = schema
            .pointer("/properties/preprocessing")
            .expect("preprocessing schema");
        assert_eq!(
            preprocessing.pointer("/default/decode_threads"),
            Some(&json!(4))
        );
        let definition = resolve_schema(&schema, preprocessing);
        assert_eq!(
            definition.pointer("/properties/decode_threads/default"),
            Some(&json!(4))
        );
        let standalone =
            flow_like_types::json::to_value(schemars::schema_for!(BarcodePreprocessingOptions))
                .expect("preprocessing schema should serialize");
        assert_eq!(
            standalone.pointer("/properties/decode_threads/default"),
            Some(&json!(4))
        );
    }

    #[test]
    fn barcode_runtime_thread_defaults_still_follow_cpu_allocation() {
        let expected = std::thread::available_parallelism()
            .map(|threads| threads.get().min(4))
            .unwrap_or(4);
        assert_eq!(
            BarcodePreprocessingOptions::default().decode_threads,
            expected
        );
        assert_eq!(
            ReadBarcodeOptions::default().preprocessing.decode_threads,
            expected
        );
        for input in [json!({}), json!({"preprocessing": {}})] {
            let options: ReadBarcodeOptions = flow_like_types::json::from_value(input).unwrap();
            assert_eq!(options.preprocessing.decode_threads, expected);
        }
        let explicit: ReadBarcodeOptions = flow_like_types::json::from_value(json!({
            "preprocessing": {"decode_threads": 7}
        }))
        .unwrap();
        assert_eq!(explicit.preprocessing.decode_threads, 7);
    }

    fn barcode_image(
        text: &str,
        format: BarcodeFormat,
        scale: u32,
        target_height: Option<u32>,
    ) -> DynamicImage {
        let hints = EncodeHints {
            Margin: Some("8".to_string()),
            ..EncodeHints::default()
        };
        let bit_matrix = MultiFormatWriter
            .encode_with_hints(text, &format, 0, 0, &hints)
            .unwrap();
        let image: DynamicImage = bit_matrix.into();
        let (width, height) = image.dimensions();
        image.resize_exact(
            width * scale,
            target_height.unwrap_or(height * scale),
            FilterType::Nearest,
        )
    }

    fn paste_luma(canvas: &mut GrayImage, image: &DynamicImage, x: u32, y: u32) {
        let luma = image.to_luma8();
        overlay(canvas, &luma, i64::from(x), i64::from(y));
    }

    #[test]
    fn decodes_mixed_formats_independently() {
        let qr_text = "qr-result";
        let code_128_text = "CODE128-123456";
        let qr = barcode_image(qr_text, BarcodeFormat::QR_CODE, 6, None);
        let code_128 = barcode_image(code_128_text, BarcodeFormat::CODE_128, 3, Some(96));
        let mut canvas: GrayImage = ImageBuffer::from_pixel(900, 320, Luma([255]));
        paste_luma(&mut canvas, &qr, 24, 24);
        paste_luma(&mut canvas, &code_128, 320, 96);

        let options = ReadBarcodeOptions {
            try_harder: true,
            expected_formats: vec!["QR_CODE".into(), "CODE_128".into()],
            preprocess: "None".into(),
            preprocessing: BarcodePreprocessingOptions {
                rotations: vec![0],
                ..BarcodePreprocessingOptions::default()
            },
            ..ReadBarcodeOptions::default()
        };
        let results = read_barcodes_in_luma(canvas.into_raw(), 900, 320, &options).unwrap();

        assert!(
            results.iter().any(|barcode| barcode.text == qr_text),
            "expected QR code in {results:?}",
        );
        assert!(
            results.iter().any(|barcode| barcode.text == code_128_text),
            "expected Code 128 barcode in {results:?}",
        );
    }

    fn decode_test_image(image: &DynamicImage, options: &ReadBarcodeOptions) -> Vec<Barcode> {
        read_canvas(&image.to_luma8(), options)
    }

    #[test]
    fn default_options_decode_vertical_and_inverted_code_128() {
        let code = barcode_image("ORIENTATION-123", BarcodeFormat::CODE_128, 3, Some(96));
        let mut canvas = GrayImage::from_pixel(900, 300, Luma([255]));
        paste_luma(&mut canvas, &code, 60, 100);
        let image = DynamicImage::ImageLuma8(canvas);
        for mode in ["None", "Balanced"] {
            let options = ReadBarcodeOptions {
                preprocess: mode.into(),
                ..Default::default()
            };
            for vertical in [false, true] {
                let original = if vertical {
                    image.rotate90()
                } else {
                    image.clone()
                };
                for inverted in [false, true] {
                    let mut input = original.clone();
                    if inverted {
                        input.invert();
                    }
                    let results = decode_test_image(&input, &options);
                    assert_eq!(
                        results.len(),
                        1,
                        "{mode}, vertical={vertical}, inverted={inverted}: {results:?}"
                    );
                    assert_eq!(results[0].text, "ORIENTATION-123");
                    assert!(results[0].points.iter().all(|point| point.x >= 0.0
                        && point.x < input.width() as f32
                        && point.y >= 0.0
                        && point.y < input.height() as f32));
                }
            }
        }
    }

    #[test]
    fn explicit_light_on_dark_works_without_preprocessing() {
        let mut image = barcode_image("LIGHT-123", BarcodeFormat::CODE_128, 3, Some(96));
        image.invert();
        let mut options = ReadBarcodeOptions {
            preprocess: "None".into(),
            also_inverted: false,
            expected_formats: vec!["CODE_128".into()],
            ..Default::default()
        };
        options.preprocessing.polarity = "LightOnDark".into();
        let results = decode_test_image(&image, &options);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "LIGHT-123");
        options.preprocessing.polarity = "DarkOnLight".into();
        assert!(decode_test_image(&image, &options).is_empty());
    }

    #[test]
    fn keeps_distinct_identical_labels_and_merges_repeat_scan_lines() {
        let code = barcode_image("SAME-123", BarcodeFormat::CODE_128, 3, Some(60));
        let options = ReadBarcodeOptions {
            expected_formats: vec!["CODE_128".into()],
            ..Default::default()
        };
        for stacked in [false, true] {
            let mut canvas = GrayImage::from_pixel(code.width() * 2 + 96, 200, Luma([255]));
            paste_luma(&mut canvas, &code, 24, 40);
            let (x, y) = if stacked {
                (24, 104)
            } else {
                (code.width() + 32, 40)
            };
            paste_luma(&mut canvas, &code, x, y);
            for vertical in [false, true] {
                let input = DynamicImage::ImageLuma8(canvas.clone());
                let input = if vertical { input.rotate90() } else { input };
                let results = decode_test_image(&input, &options);
                assert_eq!(
                    results.len(),
                    2,
                    "stacked={stacked}, vertical={vertical}: {results:?}"
                );
                assert!(results.iter().all(|barcode| barcode.text == "SAME-123"));
            }
        }
    }

    #[test]
    fn off_center_reversed_1d_keeps_source_coordinates() {
        let code = barcode_image("EDGE-123", BarcodeFormat::CODE_128, 3, Some(40));
        let options = ReadBarcodeOptions {
            preprocess: "None".into(),
            expected_formats: vec!["CODE_128".into()],
            preprocessing: BarcodePreprocessingOptions {
                rotations: vec![0],
                ..Default::default()
            },
            ..Default::default()
        };
        for reversed in [false, true] {
            let code = if reversed { code.fliph() } else { code.clone() };
            let mut canvas = GrayImage::from_pixel(900, 800, Luma([255]));
            paste_luma(&mut canvas, &code, 73, 22);
            let results = decode_test_image(&DynamicImage::ImageLuma8(canvas), &options);
            assert_eq!(results.len(), 1, "reversed={reversed}: {results:?}");
            assert_eq!(results[0].text, "EDGE-123");
            assert!(results[0].points.iter().all(|point| {
                point.x >= 73.0
                    && point.x < (73 + code.width()) as f32
                    && point.y >= 22.0
                    && point.y < 62.0
            }));
        }
    }

    #[test]
    fn balanced_finds_mixed_formats_in_both_orientations() {
        let qr = barcode_image("MIXED-QR", BarcodeFormat::QR_CODE, 5, None);
        let horizontal = barcode_image("MIXED-H", BarcodeFormat::CODE_128, 3, Some(80));
        let vertical = barcode_image("MIXED-V", BarcodeFormat::CODE_128, 3, Some(80)).rotate90();
        let mut canvas = GrayImage::from_pixel(1200, 700, Luma([255]));
        paste_luma(&mut canvas, &qr, 32, 32);
        paste_luma(&mut canvas, &horizontal, 320, 330);
        paste_luma(&mut canvas, &vertical, 1000, 120);
        let results = decode_test_image(
            &DynamicImage::ImageLuma8(canvas),
            &ReadBarcodeOptions::default(),
        );
        let mut texts: Vec<_> = results
            .iter()
            .map(|barcode| barcode.text.as_str())
            .collect();
        texts.sort_unstable();
        assert_eq!(texts, ["MIXED-H", "MIXED-QR", "MIXED-V"]);
    }

    #[test]
    fn result_limit_waits_for_validated_codes_in_other_orientations() {
        let ignored = barcode_image("IGNORE-123", BarcodeFormat::CODE_128, 3, Some(80));
        let wanted = barcode_image("WANTED-456", BarcodeFormat::CODE_128, 3, Some(80)).rotate90();
        let mut canvas = GrayImage::from_pixel(1000, 800, Luma([255]));
        paste_luma(&mut canvas, &ignored, 32, 360);
        paste_luma(&mut canvas, &wanted, 750, 120);
        let image = DynamicImage::ImageLuma8(canvas);
        for preprocess in ["Balanced", "Fallback", "None"] {
            let options = ReadBarcodeOptions {
                expected_formats: vec!["CODE_128".into()],
                preprocess: preprocess.into(),
                max_results: Some(1),
                validation: BarcodeValidationOptions {
                    text_prefix: Some("WANTED-".into()),
                    ..Default::default()
                },
                ..Default::default()
            };
            let results = decode_test_image(&image, &options);
            assert_eq!(results.len(), 1, "{preprocess}: {results:?}");
            assert_eq!(results[0].text, "WANTED-456");
        }
    }

    #[test]
    fn result_limit_returns_only_the_requested_number_of_codes() {
        let first = barcode_image("FIRST-123", BarcodeFormat::CODE_128, 3, Some(80));
        let second = barcode_image("SECOND-456", BarcodeFormat::CODE_128, 3, Some(80));
        let mut canvas = GrayImage::from_pixel(1100, 300, Luma([255]));
        paste_luma(&mut canvas, &first, 24, 100);
        paste_luma(&mut canvas, &second, 600, 100);
        let image = DynamicImage::ImageLuma8(canvas);
        let mut options = ReadBarcodeOptions {
            expected_formats: vec!["CODE_128".into()],
            max_results: Some(1),
            ..Default::default()
        };
        assert_eq!(decode_test_image(&image, &options).len(), 1);
        options.max_results = None;
        assert_eq!(decode_test_image(&image, &options).len(), 2);
    }

    #[test]
    fn read_options_schema_exposes_defaults_enums_and_limits() {
        let schema = flow_like_types::json::to_value(schemars::schema_for!(ReadBarcodeOptions))
            .expect("schema should serialize");

        let preprocess_schema = schema
            .pointer("/properties/preprocess")
            .expect("preprocess schema should exist");
        assert_eq!(preprocess_schema.get("type"), Some(&json!("string")));
        assert_eq!(
            preprocess_schema.get("enum"),
            Some(&json!([
                "None",
                "Balanced",
                "Fallback",
                "Aggressive",
                "Industrial"
            ]))
        );
        assert_eq!(preprocess_schema.get("default"), Some(&json!("Balanced")));

        let max_results_schema = &schema["properties"]["max_results"];
        assert_eq!(max_results_schema["default"], json!(null));
        assert_eq!(max_results_schema["minimum"], json!(1));

        let format_schema = schema
            .pointer("/properties/format")
            .expect("format schema should exist");
        assert_eq!(format_schema.get("default"), Some(&json!("QR_CODE")));
        assert!(
            format_schema
                .get("enum")
                .and_then(|value| value.as_array())
                .is_some_and(|formats| formats.contains(&json!("CODE_128")))
        );

        let preprocessing_schema = resolve_schema(
            &schema,
            schema
                .pointer("/properties/preprocessing")
                .expect("preprocessing schema should exist"),
        );

        let polarity_schema = preprocessing_schema
            .pointer("/properties/polarity")
            .expect("polarity schema should exist");
        assert_eq!(polarity_schema.get("default"), Some(&json!("Auto")));
        assert_eq!(
            polarity_schema.get("enum"),
            Some(&json!(["Auto", "DarkOnLight", "LightOnDark"]))
        );

        let rotations_schema = preprocessing_schema
            .pointer("/properties/rotations/items/enum")
            .expect("rotations enum should exist");
        assert_eq!(rotations_schema, &json!([0, 90, 180, 270]));

        let max_attempts_schema = preprocessing_schema
            .pointer("/properties/max_decode_attempts")
            .expect("max_decode_attempts schema should exist");
        assert_eq!(max_attempts_schema.get("default"), Some(&json!(24)));
        assert_eq!(max_attempts_schema.get("minimum"), Some(&json!(1)));
        assert_eq!(max_attempts_schema.get("maximum"), Some(&json!(256)));

        let decode_threads_schema = preprocessing_schema
            .pointer("/properties/decode_threads")
            .expect("decode_threads schema should exist");
        assert_eq!(decode_threads_schema.get("default"), Some(&json!(4)));
        assert_eq!(decode_threads_schema.get("minimum"), Some(&json!(1)));
        assert_eq!(decode_threads_schema.get("maximum"), Some(&json!(32)));
    }

    #[test]
    fn every_listed_format_parses_and_is_scanned_by_default() {
        for name in READABLE_BARCODE_FORMATS {
            let format = parse_barcode_format(name).unwrap_or_else(|error| panic!("{error}"));
            assert!(readable_barcode_formats().contains(&format), "{name}");
            assert_eq!(parse_barcode_format(&name.to_lowercase()).unwrap(), format);
        }
        assert_eq!(
            readable_barcode_formats().len(),
            READABLE_BARCODE_FORMATS.len()
        );
    }

    #[test]
    fn found_codes_name_their_format_as_the_options_do_and_box_the_symbol() {
        let qr = barcode_image("format-name", BarcodeFormat::QR_CODE, 6, None);
        let code_128 = barcode_image("CODE128-NAME", BarcodeFormat::CODE_128, 3, Some(96));
        let mut canvas = GrayImage::from_pixel(900, 320, Luma([255]));
        paste_luma(&mut canvas, &qr, 24, 24);
        paste_luma(&mut canvas, &code_128, 320, 96);
        let (width, height) = canvas.dimensions();
        let found =
            read_barcodes_in_luma(canvas.into_raw(), width, height, &Default::default()).unwrap();

        let qr = found
            .iter()
            .find(|code| code.text() == "format-name")
            .unwrap();
        assert_eq!(qr.format_name(), "QR_CODE");
        let [left, top, right, bottom] = qr.bounds().unwrap();
        assert!(left >= 20.0 && top >= 20.0 && right <= 320.0 && bottom <= 320.0);

        let code_128 = found
            .iter()
            .find(|code| code.text() == "CODE128-NAME")
            .unwrap();
        assert_eq!(code_128.format_name(), "CODE_128");
        let [left, top, right, bottom] = code_128.bounds().unwrap();
        assert!(left >= 300.0 && right > left && bottom > top);
    }

    fn read_canvas(canvas: &GrayImage, options: &ReadBarcodeOptions) -> Vec<Barcode> {
        let (width, height) = canvas.dimensions();
        read_barcodes_in_luma(canvas.as_raw().clone(), width, height, options).unwrap()
    }

    #[test]
    fn matrix_codes_away_from_the_centre_are_found_without_trying_harder() {
        let data_matrix = barcode_image("OFF-CENTRE-DM", BarcodeFormat::DATA_MATRIX, 6, None);
        let qr = barcode_image("off-centre-qr", BarcodeFormat::QR_CODE, 5, None);
        let mut canvas = GrayImage::from_pixel(1000, 1400, Luma([255]));
        paste_luma(&mut canvas, &qr, 40, 60);
        paste_luma(&mut canvas, &data_matrix, 760, 1150);
        for preprocess in ["None", "Balanced"] {
            let options = ReadBarcodeOptions {
                preprocess: preprocess.into(),
                ..Default::default()
            };
            let found = read_canvas(&canvas, &options);
            let mut texts: Vec<_> = found.iter().map(Barcode::text).collect();
            texts.sort_unstable();
            assert_eq!(texts, ["OFF-CENTRE-DM", "off-centre-qr"], "{preprocess}");
        }
    }

    #[test]
    fn a_tilted_linear_code_is_one_symbol_boxed_by_its_bars() {
        let code = barcode_image("TILTED-4711", BarcodeFormat::CODE_128, 3, Some(120));
        let mut canvas = GrayImage::from_pixel(900, 500, Luma([255]));
        paste_luma(&mut canvas, &code, 150, 190);
        let tilted = rotate_about_center(
            &canvas,
            5f32.to_radians(),
            Interpolation::Bilinear,
            Luma([255]),
        );
        for try_harder in [false, true] {
            let options = ReadBarcodeOptions {
                try_harder,
                ..Default::default()
            };
            let found = read_canvas(&tilted, &options);
            assert_eq!(found.len(), 1, "try_harder={try_harder}: {found:?}");
            assert_eq!(found[0].text, "TILTED-4711");
            assert_eq!(found[0].points.len(), 4);
            assert!(found[0].line_count >= 2);
            let [left, top, right, bottom] = found[0].bounds().unwrap();
            assert!(
                (60.0..170.0).contains(&(bottom - top)),
                "bars are 120 pixels tall: {top}..{bottom}"
            );
            assert!(right - left > code.width() as f32 * 0.7);
        }
    }

    /// A code as a phone hands it over in room light: soft, and grey on grey.
    fn blurred_and_dim(text: &str, format: BarcodeFormat, scale: u32, blur: f32) -> GrayImage {
        let code = barcode_image(text, format, scale, None);
        let mut canvas =
            GrayImage::from_pixel(code.width() + 240, code.height() + 240, Luma([255]));
        paste_luma(&mut canvas, &code, 120, 120);
        let mut dim = flow_like_types::image::imageops::blur(&canvas, blur);
        for pixel in dim.pixels_mut() {
            pixel.0[0] = 40 + (u16::from(pixel.0[0]) * 50 / 255) as u8;
        }
        dim
    }

    #[test]
    fn a_blurred_dim_code_reads_once_it_is_sharpened() {
        let dim = blurred_and_dim("blurred-and-dim", BarcodeFormat::QR_CODE, 4, 1.6);
        let unprocessed = ReadBarcodeOptions {
            preprocess: "None".into(),
            ..Default::default()
        };
        assert!(read_canvas(&dim, &unprocessed).is_empty());

        let (width, height) = dim.dimensions();
        let sharpened = sharpen_luma(dim.as_raw(), width, height);
        let sharpened = GrayImage::from_raw(width, height, sharpened).unwrap();
        let found = read_canvas(&sharpened, &unprocessed);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].text, "blurred-and-dim");

        let found = read_canvas(&dim, &ReadBarcodeOptions::default());
        assert_eq!(found.len(), 1, "the default search sharpens: {found:?}");
    }

    #[test]
    fn a_code_that_fills_the_image_reads_at_half_its_size() {
        let dim = blurred_and_dim("CLOSE-UP-DM", BarcodeFormat::DATA_MATRIX, 34, 1.6);
        let unprocessed = ReadBarcodeOptions {
            preprocess: "None".into(),
            ..Default::default()
        };
        let found = read_canvas(&dim, &unprocessed);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].text, "CLOSE-UP-DM");
        let [left, top, right, bottom] = found[0].bounds().unwrap();
        let (width, height) = (dim.width() as f32 - 240.0, dim.height() as f32 - 240.0);
        assert!(
            (right - left - width).abs() < 20.0 && (bottom - top - height).abs() < 20.0,
            "the box is the symbol's, in pixels of the full image: {left} {top} {right} {bottom}"
        );
    }
}

use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{
    anyhow, async_trait,
    json::json,
    rxing::{
        BarcodeFormat, BinaryBitmap, DecodeHints,
        Exceptions::NotFoundException,
        Luma8LuminanceSource, MultiFormatReader, RXingResult,
        common::HybridBinarizer,
        multi::{ByQuadrantReader, GenericMultipleBarcodeReader, MultipleBarcodeReader},
    },
};
#[cfg(feature = "execute")]
use rayon::prelude::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
};

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
    line_count: usize,
    points: Vec<BarcodePoint>,
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
    #[serde(default = "default_preprocess")]
    #[schemars(schema_with = "barcode_preprocess_schema")]
    pub preprocess: String,
    #[serde(default)]
    pub validation: BarcodeValidationOptions,
    #[serde(default)]
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
    #[schemars(schema_with = "decode_threads_schema")]
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
            effective.max_variants = effective.max_variants.min(1);
            effective.max_decode_attempts = effective.max_decode_attempts.min(1);
        }
        PreprocessMode::Balanced => {
            effective.contrast_stretch = true;
            effective.local_contrast = false;
            effective.denoise = false;
            effective.sharpen = false;
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

fn hash_luma(luma: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    luma.hash(&mut hasher);
    hasher.finish()
}

impl Barcode {
    fn from_result(value: RXingResult, variant: &DecodeVariant) -> Self {
        let points = value
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
            points,
        }
    }
}

impl From<RXingResult> for Barcode {
    fn from(value: RXingResult) -> Self {
        let variant = DecodeVariant {
            luma: Vec::new(),
            width: 0,
            height: 0,
            hash: 0,
            point_scale: 1.0,
            x_offset: 0,
            y_offset: 0,
            base_width: 0,
            base_height: 0,
            rotation: DecodeRotation::Deg0,
        };
        Barcode::from_result(value, &variant)
    }
}

impl DecodeVariant {
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

fn parse_barcode_format(format: &str) -> flow_like_types::Result<BarcodeFormat> {
    let normalized = format.trim().to_uppercase();
    let barcode_format = BarcodeFormat::from(normalized.as_str());
    if barcode_format == BarcodeFormat::UNSUPORTED_FORMAT
        || !READABLE_BARCODE_FORMATS
            .iter()
            .any(|supported| BarcodeFormat::from(*supported) == barcode_format)
    {
        return Err(anyhow!("Unsupported readable barcode format: {}", format));
    }
    Ok(barcode_format)
}

fn readable_barcode_formats() -> Vec<BarcodeFormat> {
    READABLE_BARCODE_FORMATS
        .iter()
        .map(|format| BarcodeFormat::from(*format))
        .filter(|format| *format != BarcodeFormat::UNSUPORTED_FORMAT)
        .collect()
}

fn push_unique_format(formats: &mut Vec<BarcodeFormat>, format: BarcodeFormat) {
    if !formats.contains(&format) {
        formats.push(format);
    }
}

fn hints_for_formats(hints: &DecodeHints, formats: &[BarcodeFormat]) -> DecodeHints {
    let mut hints = hints.clone();
    hints.PossibleFormats = Some(formats.iter().copied().collect::<HashSet<_>>());
    hints
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
            vec![0]
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

fn decode_multiple_in_luma(
    luma: &[u8],
    width: u32,
    height: u32,
    hints: &DecodeHints,
) -> flow_like_types::rxing::common::Result<Vec<RXingResult>> {
    let mut scanner = GenericMultipleBarcodeReader::new(MultiFormatReader::default());
    scanner.decode_multiple_with_hints(
        &mut BinaryBitmap::new(HybridBinarizer::new(Luma8LuminanceSource::new(
            luma.to_vec(),
            width,
            height,
        ))),
        hints,
    )
}

fn decode_multiple_in_luma_by_quadrant(
    luma: &[u8],
    width: u32,
    height: u32,
    hints: &DecodeHints,
) -> flow_like_types::rxing::common::Result<Vec<RXingResult>> {
    let mut scanner =
        GenericMultipleBarcodeReader::new(ByQuadrantReader::new(MultiFormatReader::default()));
    scanner.decode_multiple_with_hints(
        &mut BinaryBitmap::new(HybridBinarizer::new(Luma8LuminanceSource::new(
            luma.to_vec(),
            width,
            height,
        ))),
        hints,
    )
}

#[derive(Clone, Copy)]
enum ScannerMode {
    Normal,
    Quadrant,
}

struct DecodeAttemptResult {
    barcodes: Vec<Barcode>,
    error: Option<String>,
}

fn decode_variant_attempt(
    variant: &DecodeVariant,
    hints: &DecodeHints,
    scanner_mode: ScannerMode,
) -> DecodeAttemptResult {
    let decoded = match scanner_mode {
        ScannerMode::Normal => {
            decode_multiple_in_luma(&variant.luma, variant.width, variant.height, hints)
        }
        ScannerMode::Quadrant => {
            decode_multiple_in_luma_by_quadrant(&variant.luma, variant.width, variant.height, hints)
        }
    };

    match decoded {
        Ok(results) => DecodeAttemptResult {
            barcodes: results
                .into_iter()
                .map(|result| Barcode::from_result(result, variant))
                .collect(),
            error: None,
        },
        Err(NotFoundException(_)) => DecodeAttemptResult {
            barcodes: Vec::new(),
            error: None,
        },
        Err(e) => DecodeAttemptResult {
            barcodes: Vec::new(),
            error: Some(e.to_string()),
        },
    }
}

fn decode_barcodes_in_luma(
    luma: Vec<u8>,
    width: u32,
    height: u32,
    hints: &DecodeHints,
    scan_formats: &[BarcodeFormat],
    preprocess_mode: PreprocessMode,
    polarity: BarcodePolarity,
    rotations: &[DecodeRotation],
    preprocessing: &BarcodePreprocessingOptions,
) -> flow_like_types::Result<Vec<Barcode>> {
    let variants = build_decode_variants(
        luma,
        width,
        height,
        preprocess_mode,
        polarity,
        rotations,
        preprocessing,
    );
    let max_decode_attempts = preprocessing.max_decode_attempts.max(1);
    let scan_formats = if scan_formats.is_empty() {
        readable_barcode_formats()
    } else {
        scan_formats.to_vec()
    };
    let format_hints = hints_for_formats(hints, &scan_formats);

    let scanner_modes: Vec<ScannerMode> = if preprocess_mode != PreprocessMode::None {
        vec![ScannerMode::Normal, ScannerMode::Quadrant]
    } else {
        vec![ScannerMode::Normal]
    };

    let attempts: Vec<(&DecodeVariant, ScannerMode)> = variants
        .iter()
        .flat_map(|variant| {
            scanner_modes
                .iter()
                .copied()
                .map(move |mode| (variant, mode))
        })
        .take(max_decode_attempts)
        .collect();

    let decode_threads = preprocessing.decode_threads.max(1);
    let attempt_results = if decode_threads == 1 || attempts.len() <= 1 {
        attempts
            .iter()
            .map(|(variant, mode)| decode_variant_attempt(variant, &format_hints, *mode))
            .collect::<Vec<_>>()
    } else {
        #[cfg(feature = "execute")]
        {
            let thread_count = std::thread::available_parallelism()
                .map(|threads| threads.get().min(decode_threads))
                .unwrap_or(decode_threads);
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(thread_count)
                .build()
                .map_err(|e| anyhow!("Failed to build barcode decode thread pool: {}", e))?;

            pool.install(|| {
                attempts
                    .par_iter()
                    .map(|(variant, mode)| decode_variant_attempt(variant, &format_hints, *mode))
                    .collect::<Vec<_>>()
            })
        }

        #[cfg(not(feature = "execute"))]
        {
            attempts
                .iter()
                .map(|(variant, mode)| decode_variant_attempt(variant, &format_hints, *mode))
                .collect::<Vec<_>>()
        }
    };

    let mut barcodes = Vec::new();
    let mut first_error = None;
    for attempt_result in attempt_results {
        if let Some(error) = attempt_result.error {
            first_error.get_or_insert(error);
        }
        for barcode in attempt_result.barcodes {
            push_unique_barcode(&mut barcodes, barcode);
        }
    }

    if barcodes.is_empty()
        && let Some(error) = first_error
    {
        return Err(anyhow!("Decoder Error: {}", error));
    }

    Ok(barcodes)
}

fn is_exhaustive_preprocess(preprocess_mode: PreprocessMode) -> bool {
    matches!(
        preprocess_mode,
        PreprocessMode::Balanced | PreprocessMode::Aggressive | PreprocessMode::Industrial
    )
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
    let (source_luma, source_width, source_height, x_offset, y_offset) =
        crop_luma_to_roi(&luma, width, height, preprocessing.roi.as_ref());

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

    if preprocessing.contrast_stretch {
        let source_count = variants.len();
        append_processed_variants(&mut variants, source_count, max_variants, |variant| {
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
        append_processed_variants(&mut variants, source_count, max_variants, |variant| {
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
        append_processed_variants(&mut variants, base_variant_count, max_variants, |variant| {
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
        append_processed_variants(
            &mut variants,
            enhanced_variant_count,
            max_variants,
            |variant| {
                Some(variant.with_luma(
                    sharpen_luma(&variant.luma, variant.width, variant.height),
                    variant.width,
                    variant.height,
                    variant.point_scale,
                ))
            },
        );
    }

    if polarity == BarcodePolarity::LightOnDark {
        let polarity_source_count = variants.len();
        append_processed_variants(
            &mut variants,
            polarity_source_count,
            max_variants,
            |variant| {
                Some(variant.with_luma(
                    invert_luma(&variant.luma),
                    variant.width,
                    variant.height,
                    variant.point_scale,
                ))
            },
        );
    }

    let threshold_source_count = variants.len();
    if preprocessing.otsu_threshold {
        append_processed_variants(
            &mut variants,
            threshold_source_count,
            max_variants,
            |variant| {
                Some(variant.with_luma(
                    otsu_threshold_luma(&variant.luma),
                    variant.width,
                    variant.height,
                    variant.point_scale,
                ))
            },
        );
    }

    if preprocessing.adaptive_threshold {
        append_processed_variants(
            &mut variants,
            threshold_source_count,
            max_variants,
            |variant| {
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
            },
        );
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
            push_decode_variant(&mut variants, variant, max_variants);
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
        append_processed_variants(
            &mut variants,
            upscale_source_count,
            max_variants,
            |variant| {
                let (upscaled, upscaled_width, upscaled_height) = upscale_luma_nearest(
                    &variant.luma,
                    variant.width,
                    variant.height,
                    upscale_factor,
                );
                Some(variant.with_luma(
                    upscaled,
                    upscaled_width,
                    upscaled_height,
                    variant.point_scale / upscale_factor as f32,
                ))
            },
        );
    }

    variants
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
    if let Some(min_text_length) = validation.min_text_length
        && barcode.text.len() < min_text_length
    {
        return false;
    }

    if let Some(max_text_length) = validation.max_text_length
        && barcode.text.len() > max_text_length
    {
        return false;
    }

    if let Some(prefix) = &validation.text_prefix
        && !barcode.text.starts_with(prefix)
    {
        return false;
    }

    if let Some(suffix) = &validation.text_suffix
        && !barcode.text.ends_with(suffix)
    {
        return false;
    }

    if let Some(contains) = &validation.text_contains
        && !barcode.text.contains(contains)
    {
        return false;
    }

    if !validation.allowed_texts.is_empty()
        && !validation
            .allowed_texts
            .iter()
            .any(|allowed| allowed == &barcode.text)
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

fn sharpen_luma(luma: &[u8], width: u32, height: u32) -> Vec<u8> {
    if width < 3 || height < 3 {
        return luma.to_vec();
    }

    let mut output = luma.to_vec();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let center = i16::from(luma[(y * width + x) as usize]);
            let north = i16::from(luma[((y - 1) * width + x) as usize]);
            let south = i16::from(luma[((y + 1) * width + x) as usize]);
            let west = i16::from(luma[(y * width + x - 1) as usize]);
            let east = i16::from(luma[(y * width + x + 1) as usize]);
            let sharpened = (5 * center - north - south - west - east).clamp(0, 255);
            output[(y * width + x) as usize] = sharpened as u8;
        }
    }

    output
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

fn push_unique_barcode(barcodes: &mut Vec<Barcode>, candidate: Barcode) {
    if barcodes
        .iter()
        .any(|existing| is_same_barcode(existing, &candidate))
    {
        return;
    }

    barcodes.push(candidate);
}

fn is_same_barcode(a: &Barcode, b: &Barcode) -> bool {
    if a.text != b.text || a.format != b.format {
        return false;
    }

    let Some((ax, ay)) = centroid(&a.points) else {
        return true;
    };
    let Some((bx, by)) = centroid(&b.points) else {
        return true;
    };

    let dx = ax - bx;
    let dy = ay - by;
    (dx * dx + dy * dy) <= 64.0
}

fn centroid(points: &[BarcodePoint]) -> Option<(f32, f32)> {
    if points.is_empty() {
        return None;
    }

    let (sum_x, sum_y) = points
        .iter()
        .fold((0.0, 0.0), |(x, y), point| (x + point.x, y + point.y));
    let len = points.len() as f32;
    Some((sum_x / len, sum_y / len))
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
        node.set_version(6);
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

        // fetch inputs
        let options: ReadBarcodeOptions = context.evaluate_pin("options").await?;
        let preprocess_mode = parse_preprocess_mode(&options.preprocess)?;
        let preprocessing =
            effective_preprocessing_options(preprocess_mode, &options.preprocessing);
        let polarity = parse_polarity(&preprocessing.polarity)?;
        let rotations = effective_rotations(preprocess_mode, &preprocessing)?;
        let node_img: NodeImage = context.evaluate_pin("image_in").await?;

        // prepare image
        let img = node_img.get_image(context).await?;
        let (img_vec, w, h) = {
            let img_guard = img.lock().await;
            let gray = img_guard.to_luma8(); // decoding works best with grayscale images
            let (w, h) = gray.dimensions();
            (gray.into_raw(), w, h)
        };

        // detect + decode (bar)codes
        let hints = DecodeHints {
            TryHarder: Some(options.try_harder),
            AlsoInverted: Some(match polarity {
                BarcodePolarity::Auto => options.also_inverted,
                BarcodePolarity::DarkOnLight | BarcodePolarity::LightOnDark => false,
            }),
            PureBarcode: Some(options.pure_barcode),
            ..DecodeHints::default()
        };

        let mut scan_formats = Vec::new();
        for format in &options.expected_formats {
            push_unique_format(&mut scan_formats, parse_barcode_format(format)?);
        }

        if scan_formats.is_empty() && options.filter {
            let bc_type = parse_barcode_format(&options.format)?;
            push_unique_format(&mut scan_formats, bc_type);
        }

        if scan_formats.is_empty() {
            scan_formats = readable_barcode_formats();
        }

        let mut results = decode_barcodes_in_luma(
            img_vec,
            w,
            h,
            &hints,
            &scan_formats,
            preprocess_mode,
            polarity,
            &rotations,
            &preprocessing,
        )?;
        let decoded_count = results.len();
        results.retain(|barcode| barcode_matches_validation(barcode, &options.validation));
        if results.is_empty() {
            if decoded_count == 0 {
                context.log_message("No Codes Detected / Decoded!", LogLevel::Warn);
            } else {
                context.log_message(
                    "Codes were decoded, but none matched the configured validation constraints.",
                    LogLevel::Warn,
                );
            }
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
    use flow_like_types::{
        image::{
            DynamicImage, GenericImageView, GrayImage, ImageBuffer, Luma,
            imageops::{FilterType, overlay},
        },
        rxing::{EncodeHints, MultiFormatWriter, Writer},
    };

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

        let hints = DecodeHints {
            TryHarder: Some(true),
            AlsoInverted: Some(true),
            PureBarcode: Some(false),
            ..DecodeHints::default()
        };
        let preprocessing = BarcodePreprocessingOptions {
            max_decode_attempts: 8,
            max_variants: 1,
            contrast_stretch: false,
            local_contrast: false,
            denoise: false,
            otsu_threshold: false,
            adaptive_threshold: false,
            morphology: false,
            upscale_factor: 1,
            ..BarcodePreprocessingOptions::default()
        };

        let results = decode_barcodes_in_luma(
            canvas.into_raw(),
            900,
            320,
            &hints,
            &[BarcodeFormat::QR_CODE, BarcodeFormat::CODE_128],
            PreprocessMode::None,
            BarcodePolarity::Auto,
            &[DecodeRotation::Deg0],
            &preprocessing,
        )
        .unwrap();

        assert!(
            results.iter().any(|barcode| barcode.text == qr_text),
            "expected QR code in {results:?}",
        );
        assert!(
            results.iter().any(|barcode| barcode.text == code_128_text),
            "expected Code 128 barcode in {results:?}",
        );
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

        let polarity_schema = schema
            .pointer("/properties/preprocessing/properties/polarity")
            .expect("polarity schema should exist");
        assert_eq!(polarity_schema.get("default"), Some(&json!("Auto")));
        assert_eq!(
            polarity_schema.get("enum"),
            Some(&json!(["Auto", "DarkOnLight", "LightOnDark"]))
        );

        let rotations_schema = schema
            .pointer("/properties/preprocessing/properties/rotations/items/enum")
            .expect("rotations enum should exist");
        assert_eq!(rotations_schema, &json!([0, 90, 180, 270]));

        let max_attempts_schema = schema
            .pointer("/properties/preprocessing/properties/max_decode_attempts")
            .expect("max_decode_attempts schema should exist");
        assert_eq!(max_attempts_schema.get("default"), Some(&json!(24)));
        assert_eq!(max_attempts_schema.get("minimum"), Some(&json!(1)));
        assert_eq!(max_attempts_schema.get("maximum"), Some(&json!(256)));

        let decode_threads_schema = schema
            .pointer("/properties/preprocessing/properties/decode_threads")
            .expect("decode_threads schema should exist");
        assert_eq!(decode_threads_schema.get("default"), Some(&json!(4)));
        assert_eq!(decode_threads_schema.get("minimum"), Some(&json!(1)));
        assert_eq!(decode_threads_schema.get("maximum"), Some(&json!(32)));
    }
}

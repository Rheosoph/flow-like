use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod defaults {
    pub const PRIMARY: &str = "#FF4343";
    pub const PRIMARY_LIGHT: &str = "#FFF0F0";
    pub const PRIMARY_DARK: &str = "#CC3636";
    pub const TEXT: &str = "#1A1A1A";
    pub const TEXT_MUTED: &str = "#6B7280";
    pub const HEADING: &str = "#111111";
    pub const BACKGROUND: &str = "#FFFFFF";
    pub const SURFACE: &str = "#F9FAFB";
    pub const BORDER: &str = "#E5E7EB";
    pub const CODE_BG: &str = "#F8F8F8";
    pub const FONT_SANS: &str = "Calibri";
    pub const FONT_MONO: &str = "Consolas";
    pub const LINK_COLOR: &str = "#FF4343";

    pub const CHART_COLORS: &[&str] = &["#FF4343", "#FF6B6B", "#4B5563", "#9CA3AF", "#D1D5DB"];

    pub const DOCX_FONT_SIZE: f32 = 11.0;
    pub const PPTX_FONT_SIZE: f32 = 18.0;
    pub const PDF_FONT_SIZE: f32 = 11.0;
    pub const DOCX_LINE_SPACING: f32 = 1.15;
    pub const PPTX_LINE_SPACING: f32 = 1.2;
    pub const PDF_LINE_SPACING: f32 = 1.4;
    pub const MARGIN_CM: f32 = 2.54;
    pub const TABLE_CELL_PADDING_CM: f32 = 0.19;
}

/// Strip leading '#' from hex color for OpenXML attributes.
pub fn hex_to_ooxml(hex: &str) -> String {
    hex.trim_start_matches('#').to_uppercase()
}

/// Parse hex color to RGB float tuple for PDF.
pub fn hex_to_rgb(hex: &str) -> (f32, f32, f32) {
    let hex = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;
    (r, g, b)
}

/// Convert cm to EMU (English Metric Units) for PPTX.
pub fn cm_to_emu(cm: f32) -> i64 {
    (cm as f64 * 360000.0) as i64
}

/// Convert cm to twips for DOCX.
pub fn cm_to_twips(cm: f32) -> i32 {
    (cm as f64 * 567.0) as i32
}

/// Convert points to half-points for DOCX font sizes.
pub fn pt_to_half_points(pt: f32) -> i32 {
    (pt as f64 * 2.0) as i32
}

/// Convert points to hundredths of a point for PPTX font sizes.
pub fn pt_to_hundredths(pt: f32) -> i32 {
    (pt as f64 * 100.0) as i32
}

/// Convert points to EMU for PPTX border widths.
pub fn pt_to_emu(pt: f32) -> i32 {
    (pt as f64 * 12700.0) as i32
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum PageSize {
    A4,
    Letter,
    Legal,
}

impl Default for PageSize {
    fn default() -> Self {
        Self::A4
    }
}

impl PageSize {
    pub fn width_twips(&self) -> i32 {
        match self {
            Self::A4 => 11906,
            Self::Letter => 12240,
            Self::Legal => 12240,
        }
    }

    pub fn height_twips(&self) -> i32 {
        match self {
            Self::A4 => 16838,
            Self::Letter => 15840,
            Self::Legal => 20160,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum Orientation {
    Portrait,
    Landscape,
}

impl Default for Orientation {
    fn default() -> Self {
        Self::Portrait
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum TextAlignment {
    Left,
    Center,
    Right,
    Justify,
}

impl Default for TextAlignment {
    fn default() -> Self {
        Self::Left
    }
}

impl TextAlignment {
    pub fn to_ooxml_docx(&self) -> &str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Justify => "both",
        }
    }

    pub fn to_ooxml_pptx(&self) -> &str {
        match self {
            Self::Left => "l",
            Self::Center => "ctr",
            Self::Right => "r",
            Self::Justify => "just",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum ParagraphStyle {
    Normal,
    Heading1,
    Heading2,
    Heading3,
    Heading4,
    Heading5,
    Heading6,
    Title,
    Subtitle,
    Quote,
    ListBullet,
    ListNumber,
}

impl Default for ParagraphStyle {
    fn default() -> Self {
        Self::Normal
    }
}

impl ParagraphStyle {
    pub fn to_style_id(&self) -> &str {
        match self {
            Self::Normal => "Normal",
            Self::Heading1 => "Heading1",
            Self::Heading2 => "Heading2",
            Self::Heading3 => "Heading3",
            Self::Heading4 => "Heading4",
            Self::Heading5 => "Heading5",
            Self::Heading6 => "Heading6",
            Self::Title => "Title",
            Self::Subtitle => "Subtitle",
            Self::Quote => "Quote",
            Self::ListBullet => "ListBullet",
            Self::ListNumber => "ListNumber",
        }
    }

    pub fn font_size_pt(&self) -> f32 {
        match self {
            Self::Heading1 | Self::Title => 24.0,
            Self::Heading2 => 18.0,
            Self::Heading3 | Self::Subtitle => 14.0,
            Self::Heading4 => 12.0,
            Self::Heading5 => 11.0,
            Self::Heading6 => 10.0,
            _ => defaults::DOCX_FONT_SIZE,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum ShapeType {
    Rectangle,
    RoundedRectangle,
    Ellipse,
    Triangle,
    Arrow,
    Line,
}

impl Default for ShapeType {
    fn default() -> Self {
        Self::Rectangle
    }
}

impl ShapeType {
    pub fn to_pptx_preset(&self) -> &str {
        match self {
            Self::Rectangle => "rect",
            Self::RoundedRectangle => "roundRect",
            Self::Ellipse => "ellipse",
            Self::Triangle => "triangle",
            Self::Arrow => "rightArrow",
            Self::Line => "line",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum PageNumberPosition {
    BottomCenter,
    BottomRight,
    BottomLeft,
    TopCenter,
    TopRight,
}

impl Default for PageNumberPosition {
    fn default() -> Self {
        Self::BottomCenter
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum Rotation {
    #[serde(rename = "0")]
    None,
    #[serde(rename = "90")]
    Clockwise90,
    #[serde(rename = "180")]
    Rotate180,
    #[serde(rename = "270")]
    Clockwise270,
}

impl Default for Rotation {
    fn default() -> Self {
        Self::None
    }
}

impl Rotation {
    pub fn degrees(&self) -> i32 {
        match self {
            Self::None => 0,
            Self::Clockwise90 => 90,
            Self::Rotate180 => 180,
            Self::Clockwise270 => 270,
        }
    }
}

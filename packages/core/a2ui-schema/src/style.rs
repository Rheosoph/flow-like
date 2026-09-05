use flow_like_types_proto::proto;
use schemars::JsonSchema;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Helper to deserialize either a string or an object with a value field
fn deserialize_string_or_value<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    struct StringOrValueVisitor;

    impl<'de> Visitor<'de> for StringOrValueVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a string or an object with a 'value' field")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v.to_string())
        }

        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(v)
        }

        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut value: Option<String> = None;
            while let Some(key) = map.next_key::<String>()? {
                if key == "value" {
                    value = Some(map.next_value()?);
                } else {
                    let _: serde::de::IgnoredAny = map.next_value()?;
                }
            }
            value.ok_or_else(|| de::Error::missing_field("value"))
        }
    }

    deserializer.deserialize_any(StringOrValueVisitor)
}

/// Split a CSS shorthand value without breaking whitespace inside functions
/// such as `calc(...)`, `rgba(...)`, or `var(...)`.
fn split_css_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut depth = 0_u32;

    for character in value.chars() {
        match character {
            '(' => {
                depth += 1;
                current.push(character);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(character);
            }
            character if character.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn is_shadow_length(value: &str) -> bool {
    value == "0"
        || value.starts_with("calc(")
        || value.starts_with("min(")
        || value.starts_with("max(")
        || value.starts_with("clamp(")
        || value.chars().next().is_some_and(|character| {
            character.is_ascii_digit() || matches!(character, '-' | '+' | '.')
        })
}

/// Overflow behavior
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Overflow {
    #[default]
    Visible,
    Hidden,
    Scroll,
    Auto,
}

impl From<proto::Overflow> for Overflow {
    fn from(proto: proto::Overflow) -> Self {
        match proto {
            proto::Overflow::Visible => Self::Visible,
            proto::Overflow::Hidden => Self::Hidden,
            proto::Overflow::Scroll => Self::Scroll,
            proto::Overflow::Auto => Self::Auto,
        }
    }
}

impl From<Overflow> for proto::Overflow {
    fn from(value: Overflow) -> Self {
        match value {
            Overflow::Visible => proto::Overflow::Visible,
            Overflow::Hidden => proto::Overflow::Hidden,
            Overflow::Scroll => proto::Overflow::Scroll,
            Overflow::Auto => proto::Overflow::Auto,
        }
    }
}

/// Background type
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Background {
    Color(String),
    Gradient(Gradient),
    Image(BackgroundImage),
    Blur(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundImage {
    pub url: super::BoundValue,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Gradient {
    #[serde(rename = "type")]
    pub gradient_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub angle: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    pub stops: Vec<GradientStop>,
}

impl<'de> Deserialize<'de> for Gradient {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct GradientWire {
            #[serde(rename = "type")]
            frontend_type: Option<String>,
            #[serde(rename = "gradientType")]
            rust_type: Option<String>,
            angle: Option<f32>,
            direction: Option<String>,
            stops: Vec<GradientStop>,
        }

        let wire = GradientWire::deserialize(deserializer)?;
        let legacy_rust_shape = wire.frontend_type.is_none() && wire.rust_type.is_some();
        let gradient_type = wire
            .frontend_type
            .or(wire.rust_type)
            .ok_or_else(|| de::Error::missing_field("type"))?;
        let mut stops = wire.stops;

        // The previous Rust contract documented stop positions as 0.0..=1.0,
        // while the established frontend contract uses CSS percentages. The
        // legacy key tells us when that conversion is safe to perform.
        if legacy_rust_shape {
            for stop in &mut stops {
                if let Some(position) = stop.position.as_mut()
                    && (0.0..=1.0).contains(position)
                {
                    *position *= 100.0;
                }
            }
        }

        Ok(Self {
            gradient_type,
            angle: wire.angle,
            direction: wire.direction,
            stops,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GradientStop {
    pub color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<f32>,
}

/// Border styling
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Border {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<String>,
}

impl Border {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_width(mut self, width: impl Into<String>) -> Self {
        self.width = Some(width.into());
        self
    }

    pub fn with_radius(mut self, radius: impl Into<String>) -> Self {
        self.radius = Some(radius.into());
        self
    }

    pub fn to_css(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref width) = self.width {
            parts.push(format!("border-width: {};", width));
        }
        if let Some(ref style) = self.style {
            parts.push(format!("border-style: {};", style));
        }
        if let Some(ref color) = self.color {
            parts.push(format!("border-color: {};", color));
        }
        if let Some(ref radius) = self.radius {
            parts.push(format!("border-radius: {};", radius));
        }

        parts.join(" ")
    }
}

/// Shadow styling. The JSON representation intentionally matches the original
/// frontend contract. The previous `{ boxShadows, textShadow }` shape is still
/// accepted during deserialization and normalized to these fields.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Shadow {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blur: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spread: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inset: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_shadow: Option<String>,
}

impl Shadow {
    fn has_box_shadow(&self) -> bool {
        self.x.is_some()
            || self.y.is_some()
            || self.blur.is_some()
            || self.spread.is_some()
            || self.color.is_some()
            || self.inset.is_some()
    }

    fn box_shadow_css(&self) -> Option<String> {
        if !self.has_box_shadow() {
            return None;
        }

        Some(
            [
                self.inset.unwrap_or(false).then_some("inset".to_string()),
                Some(self.x.clone().unwrap_or_else(|| "0".to_string())),
                Some(self.y.clone().unwrap_or_else(|| "0".to_string())),
                Some(self.blur.clone().unwrap_or_else(|| "0".to_string())),
                Some(self.spread.clone().unwrap_or_else(|| "0".to_string())),
                Some(
                    self.color
                        .clone()
                        .unwrap_or_else(|| "rgba(0,0,0,0.25)".to_string()),
                ),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" "),
        )
    }

    fn from_box_shadow_css(value: &str) -> Self {
        let mut tokens = split_css_tokens(value);
        let inset = tokens
            .iter()
            .position(|token| token.eq_ignore_ascii_case("inset"))
            .map(|index| {
                tokens.remove(index);
                true
            });

        if tokens.len() < 2 {
            return Self::default();
        }

        let x = Some(tokens.remove(0));
        let y = Some(tokens.remove(0));
        let blur = tokens
            .first()
            .filter(|token| is_shadow_length(token))
            .cloned()
            .inspect(|_token| {
                tokens.remove(0);
            });
        let spread = tokens
            .first()
            .filter(|token| is_shadow_length(token))
            .cloned()
            .inspect(|_token| {
                tokens.remove(0);
            });
        let color = (!tokens.is_empty()).then(|| tokens.join(" "));

        Self {
            x,
            y,
            blur,
            spread,
            color,
            inset,
            text_shadow: None,
        }
    }

    pub fn to_css(&self) -> String {
        let mut parts = Vec::new();

        if let Some(box_shadow) = self.box_shadow_css() {
            parts.push(format!("box-shadow: {};", box_shadow));
        }
        if let Some(ref text) = self.text_shadow {
            parts.push(format!("text-shadow: {};", text));
        }

        parts.join(" ")
    }
}

impl<'de> Deserialize<'de> for Shadow {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ShadowWire {
            x: Option<String>,
            y: Option<String>,
            blur: Option<String>,
            spread: Option<String>,
            color: Option<String>,
            inset: Option<bool>,
            #[serde(default)]
            box_shadows: Vec<String>,
            text_shadow: Option<String>,
        }

        let wire = ShadowWire::deserialize(deserializer)?;
        let has_frontend_shape = wire.x.is_some()
            || wire.y.is_some()
            || wire.blur.is_some()
            || wire.spread.is_some()
            || wire.color.is_some()
            || wire.inset.is_some();

        let mut shadow = if has_frontend_shape {
            Self {
                x: wire.x,
                y: wire.y,
                blur: wire.blur,
                spread: wire.spread,
                color: wire.color,
                inset: wire.inset,
                text_shadow: None,
            }
        } else {
            wire.box_shadows
                .first()
                .map(|value| Self::from_box_shadow_css(value))
                .unwrap_or_default()
        };
        shadow.text_shadow = wire.text_shadow;
        Ok(shadow)
    }
}

/// Spacing (padding/margin). Edge fields are the canonical frontend JSON shape;
/// strings and `{ "value": "..." }` remain accepted for compatibility.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Spacing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<String>,
}

impl<'de> Deserialize<'de> for Spacing {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum SpacingWire {
            String(String),
            Value {
                value: String,
            },
            Edges {
                top: Option<String>,
                right: Option<String>,
                bottom: Option<String>,
                left: Option<String>,
            },
        }

        match SpacingWire::deserialize(deserializer)? {
            SpacingWire::String(value) | SpacingWire::Value { value } => {
                Ok(Self::from_shorthand(&value))
            }
            SpacingWire::Edges {
                top,
                right,
                bottom,
                left,
            } => Ok(Self {
                top,
                right,
                bottom,
                left,
            }),
        }
    }
}

impl Spacing {
    pub fn new(value: impl Into<String>) -> Self {
        Self::from_shorthand(&value.into())
    }

    fn from_shorthand(value: &str) -> Self {
        let tokens = split_css_tokens(value);
        let (top, right, bottom, left) = match tokens.as_slice() {
            [all] => (all.clone(), all.clone(), all.clone(), all.clone()),
            [vertical, horizontal] => (
                vertical.clone(),
                horizontal.clone(),
                vertical.clone(),
                horizontal.clone(),
            ),
            [top, horizontal, bottom] => (
                top.clone(),
                horizontal.clone(),
                bottom.clone(),
                horizontal.clone(),
            ),
            [top, right, bottom, left] => {
                (top.clone(), right.clone(), bottom.clone(), left.clone())
            }
            _ => (
                value.to_string(),
                value.to_string(),
                value.to_string(),
                value.to_string(),
            ),
        };

        Self {
            top: Some(top),
            right: Some(right),
            bottom: Some(bottom),
            left: Some(left),
        }
    }

    fn to_css(&self, property: &str) -> String {
        if let Some(value) = self.as_shorthand() {
            return format!("{}: {};", property, value);
        }

        [
            ("top", &self.top),
            ("right", &self.right),
            ("bottom", &self.bottom),
            ("left", &self.left),
        ]
        .into_iter()
        .filter_map(|(edge, value)| {
            value
                .as_ref()
                .map(|value| format!("{}-{}: {};", property, edge, value))
        })
        .collect::<Vec<_>>()
        .join(" ")
    }

    fn as_shorthand(&self) -> Option<String> {
        match (&self.top, &self.right, &self.bottom, &self.left) {
            (Some(top), Some(right), Some(bottom), Some(left)) => {
                Some(format!("{} {} {} {}", top, right, bottom, left))
            }
            _ => None,
        }
    }
}

impl From<&str> for Spacing {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

/// Size value - accepts both "20px" and { "value": "20px" }
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct Size {
    pub value: String,
}

impl<'de> Deserialize<'de> for Size {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = deserialize_string_or_value(deserializer)?;
        Ok(Size { value })
    }
}

impl Size {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }

    pub fn px(value: i32) -> Self {
        Self::new(format!("{}px", value))
    }

    pub fn percent(value: i32) -> Self {
        Self::new(format!("{}%", value))
    }

    pub fn auto() -> Self {
        Self::new("auto")
    }

    pub fn full() -> Self {
        Self::new("100%")
    }
}

/// Position styling
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bottom: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<String>,
    #[serde(rename = "type", alias = "positionType")]
    pub position_type: String,
}

impl Position {
    pub fn absolute() -> Self {
        Self {
            position_type: "absolute".to_string(),
            ..Default::default()
        }
    }

    pub fn relative() -> Self {
        Self {
            position_type: "relative".to_string(),
            ..Default::default()
        }
    }

    pub fn fixed() -> Self {
        Self {
            position_type: "fixed".to_string(),
            ..Default::default()
        }
    }

    pub fn with_top(mut self, top: impl Into<String>) -> Self {
        self.top = Some(top.into());
        self
    }

    pub fn with_right(mut self, right: impl Into<String>) -> Self {
        self.right = Some(right.into());
        self
    }

    pub fn with_bottom(mut self, bottom: impl Into<String>) -> Self {
        self.bottom = Some(bottom.into());
        self
    }

    pub fn with_left(mut self, left: impl Into<String>) -> Self {
        self.left = Some(left.into());
        self
    }

    pub fn to_css(&self) -> String {
        let mut parts = vec![format!("position: {};", self.position_type)];

        if let Some(ref top) = self.top {
            parts.push(format!("top: {};", top));
        }
        if let Some(ref right) = self.right {
            parts.push(format!("right: {};", right));
        }
        if let Some(ref bottom) = self.bottom {
            parts.push(format!("bottom: {};", bottom));
        }
        if let Some(ref left) = self.left {
            parts.push(format!("left: {};", left));
        }

        parts.join(" ")
    }
}

/// Transform styling
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Transform {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotate: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform_origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skew: Option<String>,
}

impl Transform {
    pub fn to_css(&self) -> String {
        let mut transforms = Vec::new();

        if let Some(ref t) = self.translate {
            transforms.push(format!("translate({})", t));
        }
        if let Some(r) = self.rotate {
            transforms.push(format!("rotate({}deg)", r));
        }
        if let Some(ref s) = self.scale {
            transforms.push(format!("scale({})", s));
        }
        if let Some(ref sk) = self.skew {
            transforms.push(format!("skew({})", sk));
        }

        if transforms.is_empty() {
            return String::new();
        }

        let mut result = format!("transform: {};", transforms.join(" "));
        if let Some(ref origin) = self.transform_origin {
            result.push_str(&format!(" transform-origin: {};", origin));
        }

        result
    }
}

/// Breakpoint style overrides
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BreakpointStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flex_direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justify_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align_items: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grid_cols: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<Size>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<Size>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<Spacing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin: Option<Spacing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_align: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<i32>,
}

/// Responsive overrides for different breakpoints
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ResponsiveOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sm: Option<BreakpointStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub md: Option<BreakpointStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lg: Option<BreakpointStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xl: Option<BreakpointStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xxl: Option<BreakpointStyle>,
}

/// Complete style definition
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Style {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<Background>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub border: Option<Border>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<Spacing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin: Option<Spacing>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<Size>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<Size>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_width: Option<Size>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_width: Option<Size>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_height: Option<Size>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_height: Option<Size>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub z_index: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overflow: Option<Overflow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(
        rename = "responsiveOverrides",
        alias = "responsive",
        skip_serializing_if = "Option::is_none"
    )]
    pub responsive: Option<ResponsiveOverrides>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flex_grow: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flex_shrink: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flex_basis: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align_self: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grid_column: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grid_row: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grid_area: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justify_self: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gap: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_height: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub letter_spacing: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_align: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_decoration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_transform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub white_space: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_break: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_select: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pointer_events: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub animation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_offset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backdrop_filter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
}

impl Style {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_class(mut self, class_name: impl Into<String>) -> Self {
        self.class_name = Some(class_name.into());
        self
    }

    pub fn with_padding(mut self, padding: impl Into<Spacing>) -> Self {
        self.padding = Some(padding.into());
        self
    }

    pub fn with_margin(mut self, margin: impl Into<Spacing>) -> Self {
        self.margin = Some(margin.into());
        self
    }

    pub fn with_width(mut self, width: Size) -> Self {
        self.width = Some(width);
        self
    }

    pub fn with_height(mut self, height: Size) -> Self {
        self.height = Some(height);
        self
    }

    pub fn to_tailwind_classes(&self) -> String {
        let mut classes = Vec::new();

        if let Some(ref cn) = self.class_name {
            classes.push(cn.clone());
        }

        classes.join(" ")
    }

    pub fn to_inline_css(&self) -> String {
        let mut styles = Vec::new();

        if let Some(ref padding) = self.padding {
            let css = padding.to_css("padding");
            if !css.is_empty() {
                styles.push(css);
            }
        }
        if let Some(ref margin) = self.margin {
            let css = margin.to_css("margin");
            if !css.is_empty() {
                styles.push(css);
            }
        }
        if let Some(ref width) = self.width {
            styles.push(format!("width: {};", width.value));
        }
        if let Some(ref height) = self.height {
            styles.push(format!("height: {};", height.value));
        }
        if let Some(ref min_width) = self.min_width {
            styles.push(format!("min-width: {};", min_width.value));
        }
        if let Some(ref max_width) = self.max_width {
            styles.push(format!("max-width: {};", max_width.value));
        }
        if let Some(ref min_height) = self.min_height {
            styles.push(format!("min-height: {};", min_height.value));
        }
        if let Some(ref max_height) = self.max_height {
            styles.push(format!("max-height: {};", max_height.value));
        }
        if let Some(z) = self.z_index {
            styles.push(format!("z-index: {};", z));
        }
        if let Some(o) = self.opacity {
            styles.push(format!("opacity: {};", o));
        }
        if let Some(ref cursor) = self.cursor {
            styles.push(format!("cursor: {};", cursor));
        }
        if let Some(ref border) = self.border {
            let border_css = border.to_css();
            if !border_css.is_empty() {
                styles.push(border_css);
            }
        }
        if let Some(ref shadow) = self.shadow {
            let shadow_css = shadow.to_css();
            if !shadow_css.is_empty() {
                styles.push(shadow_css);
            }
        }
        if let Some(ref position) = self.position {
            styles.push(position.to_css());
        }
        if let Some(ref transform) = self.transform {
            let transform_css = transform.to_css();
            if !transform_css.is_empty() {
                styles.push(transform_css);
            }
        }
        if let Some(ref overflow) = self.overflow {
            let overflow_str = match overflow {
                Overflow::Visible => "visible",
                Overflow::Hidden => "hidden",
                Overflow::Scroll => "scroll",
                Overflow::Auto => "auto",
            };
            styles.push(format!("overflow: {};", overflow_str));
        }
        if let Some(ref flex) = self.flex {
            styles.push(format!("flex: {};", flex));
        }
        if let Some(flex_grow) = self.flex_grow {
            styles.push(format!("flex-grow: {};", flex_grow));
        }
        if let Some(flex_shrink) = self.flex_shrink {
            styles.push(format!("flex-shrink: {};", flex_shrink));
        }
        if let Some(ref flex_basis) = self.flex_basis {
            styles.push(format!("flex-basis: {};", flex_basis));
        }
        if let Some(ref align_self) = self.align_self {
            styles.push(format!("align-self: {};", align_self));
        }
        if let Some(ref grid_column) = self.grid_column {
            styles.push(format!("grid-column: {};", grid_column));
        }
        if let Some(ref grid_row) = self.grid_row {
            styles.push(format!("grid-row: {};", grid_row));
        }
        if let Some(ref grid_area) = self.grid_area {
            styles.push(format!("grid-area: {};", grid_area));
        }
        if let Some(ref justify_self) = self.justify_self {
            styles.push(format!("justify-self: {};", justify_self));
        }
        if let Some(ref gap) = self.gap {
            styles.push(format!("gap: {};", gap));
        }

        for (property, value) in [
            ("color", &self.color),
            ("font-size", &self.font_size),
            ("font-weight", &self.font_weight),
            ("font-family", &self.font_family),
            ("line-height", &self.line_height),
            ("letter-spacing", &self.letter_spacing),
            ("text-align", &self.text_align),
            ("text-decoration", &self.text_decoration),
            ("text-transform", &self.text_transform),
            ("white-space", &self.white_space),
            ("word-break", &self.word_break),
            ("visibility", &self.visibility),
            ("user-select", &self.user_select),
            ("pointer-events", &self.pointer_events),
            ("transition", &self.transition),
            ("animation", &self.animation),
            ("display", &self.display),
            ("outline", &self.outline),
            ("outline-offset", &self.outline_offset),
            ("filter", &self.filter),
            ("backdrop-filter", &self.backdrop_filter),
            ("aspect-ratio", &self.aspect_ratio),
        ] {
            if let Some(value) = value {
                styles.push(format!("{}: {};", property, value));
            }
        }

        styles.join(" ")
    }

    pub fn merge_with(&self, other: &Style) -> Style {
        Style {
            class_name: other.class_name.clone().or_else(|| self.class_name.clone()),
            background: other.background.clone().or_else(|| self.background.clone()),
            border: other.border.clone().or_else(|| self.border.clone()),
            shadow: other.shadow.clone().or_else(|| self.shadow.clone()),
            padding: other.padding.clone().or_else(|| self.padding.clone()),
            margin: other.margin.clone().or_else(|| self.margin.clone()),
            width: other.width.clone().or_else(|| self.width.clone()),
            height: other.height.clone().or_else(|| self.height.clone()),
            min_width: other.min_width.clone().or_else(|| self.min_width.clone()),
            max_width: other.max_width.clone().or_else(|| self.max_width.clone()),
            min_height: other.min_height.clone().or_else(|| self.min_height.clone()),
            max_height: other.max_height.clone().or_else(|| self.max_height.clone()),
            position: other.position.clone().or_else(|| self.position.clone()),
            z_index: other.z_index.or(self.z_index),
            transform: other.transform.clone().or_else(|| self.transform.clone()),
            opacity: other.opacity.or(self.opacity),
            overflow: other.overflow.or(self.overflow),
            cursor: other.cursor.clone().or_else(|| self.cursor.clone()),
            responsive: other.responsive.clone().or_else(|| self.responsive.clone()),
            flex: other.flex.clone().or_else(|| self.flex.clone()),
            flex_grow: other.flex_grow.or(self.flex_grow),
            flex_shrink: other.flex_shrink.or(self.flex_shrink),
            flex_basis: other.flex_basis.clone().or_else(|| self.flex_basis.clone()),
            align_self: other.align_self.clone().or_else(|| self.align_self.clone()),
            grid_column: other
                .grid_column
                .clone()
                .or_else(|| self.grid_column.clone()),
            grid_row: other.grid_row.clone().or_else(|| self.grid_row.clone()),
            grid_area: other.grid_area.clone().or_else(|| self.grid_area.clone()),
            justify_self: other
                .justify_self
                .clone()
                .or_else(|| self.justify_self.clone()),
            gap: other.gap.clone().or_else(|| self.gap.clone()),
            color: other.color.clone().or_else(|| self.color.clone()),
            font_size: other.font_size.clone().or_else(|| self.font_size.clone()),
            font_weight: other
                .font_weight
                .clone()
                .or_else(|| self.font_weight.clone()),
            font_family: other
                .font_family
                .clone()
                .or_else(|| self.font_family.clone()),
            line_height: other
                .line_height
                .clone()
                .or_else(|| self.line_height.clone()),
            letter_spacing: other
                .letter_spacing
                .clone()
                .or_else(|| self.letter_spacing.clone()),
            text_align: other.text_align.clone().or_else(|| self.text_align.clone()),
            text_decoration: other
                .text_decoration
                .clone()
                .or_else(|| self.text_decoration.clone()),
            text_transform: other
                .text_transform
                .clone()
                .or_else(|| self.text_transform.clone()),
            white_space: other
                .white_space
                .clone()
                .or_else(|| self.white_space.clone()),
            word_break: other.word_break.clone().or_else(|| self.word_break.clone()),
            visibility: other.visibility.clone().or_else(|| self.visibility.clone()),
            user_select: other
                .user_select
                .clone()
                .or_else(|| self.user_select.clone()),
            pointer_events: other
                .pointer_events
                .clone()
                .or_else(|| self.pointer_events.clone()),
            transition: other.transition.clone().or_else(|| self.transition.clone()),
            animation: other.animation.clone().or_else(|| self.animation.clone()),
            display: other.display.clone().or_else(|| self.display.clone()),
            outline: other.outline.clone().or_else(|| self.outline.clone()),
            outline_offset: other
                .outline_offset
                .clone()
                .or_else(|| self.outline_offset.clone()),
            filter: other.filter.clone().or_else(|| self.filter.clone()),
            backdrop_filter: other
                .backdrop_filter
                .clone()
                .or_else(|| self.backdrop_filter.clone()),
            aspect_ratio: other
                .aspect_ratio
                .clone()
                .or_else(|| self.aspect_ratio.clone()),
        }
    }
}

// ============================================================================
// Proto Conversions
// ============================================================================

impl From<Background> for proto::Background {
    fn from(value: Background) -> Self {
        proto::Background {
            background_type: Some(match value {
                Background::Color(c) => proto::background::BackgroundType::Color(c),
                Background::Gradient(g) => proto::background::BackgroundType::Gradient(g.into()),
                Background::Image(i) => proto::background::BackgroundType::Image(i.into()),
                Background::Blur(b) => proto::background::BackgroundType::Blur(b),
            }),
        }
    }
}

impl From<proto::Background> for Background {
    fn from(proto: proto::Background) -> Self {
        match proto.background_type {
            Some(proto::background::BackgroundType::Color(c)) => Background::Color(c),
            Some(proto::background::BackgroundType::Gradient(g)) => Background::Gradient(g.into()),
            Some(proto::background::BackgroundType::Image(i)) => Background::Image(i.into()),
            Some(proto::background::BackgroundType::Blur(b)) => Background::Blur(b),
            None => Background::Color(String::new()),
        }
    }
}

impl From<Gradient> for proto::Gradient {
    fn from(value: Gradient) -> Self {
        proto::Gradient {
            gradient_type: value.gradient_type,
            direction: value.direction,
            stops: value.stops.into_iter().map(Into::into).collect(),
            angle: value.angle,
            stop_positions_are_percent: Some(true),
        }
    }
}

impl From<proto::Gradient> for Gradient {
    fn from(proto: proto::Gradient) -> Self {
        let positions_are_percent = proto.stop_positions_are_percent.unwrap_or(false);
        let mut stops: Vec<GradientStop> = proto.stops.into_iter().map(Into::into).collect();
        if !positions_are_percent {
            for stop in &mut stops {
                if let Some(position) = stop.position.as_mut()
                    && (0.0..=1.0).contains(position)
                {
                    *position *= 100.0;
                }
            }
        }

        Gradient {
            gradient_type: proto.gradient_type,
            angle: proto.angle,
            direction: proto.direction,
            stops,
        }
    }
}

impl From<GradientStop> for proto::GradientStop {
    fn from(value: GradientStop) -> Self {
        proto::GradientStop {
            color: value.color,
            position: value.position,
        }
    }
}

impl From<proto::GradientStop> for GradientStop {
    fn from(proto: proto::GradientStop) -> Self {
        GradientStop {
            color: proto.color,
            position: proto.position,
        }
    }
}

impl From<BackgroundImage> for proto::BackgroundImage {
    fn from(value: BackgroundImage) -> Self {
        proto::BackgroundImage {
            url: Some(value.url.into()),
            size: value.size,
            position: value.position,
            repeat: value.repeat,
        }
    }
}

impl From<proto::BackgroundImage> for BackgroundImage {
    fn from(proto: proto::BackgroundImage) -> Self {
        BackgroundImage {
            url: proto
                .url
                .map(|u| (&u).into())
                .unwrap_or(super::BoundValue::literal_string("")),
            size: proto.size,
            position: proto.position,
            repeat: proto.repeat,
        }
    }
}

impl From<Border> for proto::Border {
    fn from(value: Border) -> Self {
        proto::Border {
            width: value.width,
            style: value.style,
            color: value.color,
            radius: value.radius,
        }
    }
}

impl From<proto::Border> for Border {
    fn from(proto: proto::Border) -> Self {
        Border {
            width: proto.width,
            style: proto.style,
            color: proto.color,
            radius: proto.radius,
        }
    }
}

impl From<Shadow> for proto::Shadow {
    fn from(value: Shadow) -> Self {
        let box_shadows = value.box_shadow_css().into_iter().collect();
        proto::Shadow {
            box_shadows,
            text_shadow: value.text_shadow,
            x: value.x,
            y: value.y,
            blur: value.blur,
            spread: value.spread,
            color: value.color,
            inset: value.inset,
        }
    }
}

impl From<proto::Shadow> for Shadow {
    fn from(proto: proto::Shadow) -> Self {
        let has_frontend_shape = proto.x.is_some()
            || proto.y.is_some()
            || proto.blur.is_some()
            || proto.spread.is_some()
            || proto.color.is_some()
            || proto.inset.is_some();
        let mut shadow = if has_frontend_shape {
            Shadow {
                x: proto.x,
                y: proto.y,
                blur: proto.blur,
                spread: proto.spread,
                color: proto.color,
                inset: proto.inset,
                text_shadow: None,
            }
        } else {
            proto
                .box_shadows
                .first()
                .map(|value| Shadow::from_box_shadow_css(value))
                .unwrap_or_default()
        };
        shadow.text_shadow = proto.text_shadow;
        shadow
    }
}

impl From<Spacing> for proto::Spacing {
    fn from(value: Spacing) -> Self {
        let legacy_value = value.as_shorthand();
        proto::Spacing {
            value: legacy_value,
            top: value.top,
            right: value.right,
            bottom: value.bottom,
            left: value.left,
        }
    }
}

impl From<proto::Spacing> for Spacing {
    fn from(proto: proto::Spacing) -> Self {
        let has_edges = proto.top.is_some()
            || proto.right.is_some()
            || proto.bottom.is_some()
            || proto.left.is_some();
        if has_edges {
            Spacing {
                top: proto.top,
                right: proto.right,
                bottom: proto.bottom,
                left: proto.left,
            }
        } else {
            proto
                .value
                .as_deref()
                .map(Spacing::from_shorthand)
                .unwrap_or_default()
        }
    }
}

impl From<Size> for proto::Size {
    fn from(value: Size) -> Self {
        proto::Size { value: value.value }
    }
}

impl From<proto::Size> for Size {
    fn from(proto: proto::Size) -> Self {
        Size { value: proto.value }
    }
}

impl From<Position> for proto::Position {
    fn from(value: Position) -> Self {
        proto::Position {
            top: value.top,
            right: value.right,
            bottom: value.bottom,
            left: value.left,
            position_type: value.position_type,
        }
    }
}

impl From<proto::Position> for Position {
    fn from(proto: proto::Position) -> Self {
        Position {
            top: proto.top,
            right: proto.right,
            bottom: proto.bottom,
            left: proto.left,
            position_type: proto.position_type,
        }
    }
}

impl From<Transform> for proto::Transform {
    fn from(value: Transform) -> Self {
        proto::Transform {
            translate: value.translate,
            rotate: value.rotate,
            scale: value.scale,
            transform_origin: value.transform_origin,
            skew: value.skew,
        }
    }
}

impl From<proto::Transform> for Transform {
    fn from(proto: proto::Transform) -> Self {
        Transform {
            translate: proto.translate,
            rotate: proto.rotate,
            scale: proto.scale,
            transform_origin: proto.transform_origin,
            skew: proto.skew,
        }
    }
}

impl From<Style> for proto::Style {
    fn from(value: Style) -> Self {
        proto::Style {
            class_name: value.class_name,
            background: value.background.map(Into::into),
            border: value.border.map(Into::into),
            shadow: value.shadow.map(Into::into),
            padding: value.padding.map(Into::into),
            margin: value.margin.map(Into::into),
            width: value.width.map(Into::into),
            height: value.height.map(Into::into),
            min_width: value.min_width.map(Into::into),
            max_width: value.max_width.map(Into::into),
            min_height: value.min_height.map(Into::into),
            max_height: value.max_height.map(Into::into),
            position: value.position.map(Into::into),
            z_index: value.z_index,
            transform: value.transform.map(Into::into),
            opacity: value.opacity,
            overflow: value.overflow.map(|o| proto::Overflow::from(o) as i32),
            cursor: value.cursor,
            responsive: value.responsive.map(Into::into),
            flex: value.flex,
            flex_grow: value.flex_grow,
            flex_shrink: value.flex_shrink,
            flex_basis: value.flex_basis,
            align_self: value.align_self,
            grid_column: value.grid_column,
            grid_row: value.grid_row,
            grid_area: value.grid_area,
            justify_self: value.justify_self,
            gap: value.gap,
            color: value.color,
            font_size: value.font_size,
            font_weight: value.font_weight,
            font_family: value.font_family,
            line_height: value.line_height,
            letter_spacing: value.letter_spacing,
            text_align: value.text_align,
            text_decoration: value.text_decoration,
            text_transform: value.text_transform,
            white_space: value.white_space,
            word_break: value.word_break,
            visibility: value.visibility,
            user_select: value.user_select,
            pointer_events: value.pointer_events,
            transition: value.transition,
            animation: value.animation,
            display: value.display,
            outline: value.outline,
            outline_offset: value.outline_offset,
            filter: value.filter,
            backdrop_filter: value.backdrop_filter,
            aspect_ratio: value.aspect_ratio,
        }
    }
}

impl From<proto::Style> for Style {
    fn from(proto: proto::Style) -> Self {
        Style {
            class_name: proto.class_name,
            background: proto.background.map(Into::into),
            border: proto.border.map(Into::into),
            shadow: proto.shadow.map(Into::into),
            padding: proto.padding.map(Into::into),
            margin: proto.margin.map(Into::into),
            width: proto.width.map(Into::into),
            height: proto.height.map(Into::into),
            min_width: proto.min_width.map(Into::into),
            max_width: proto.max_width.map(Into::into),
            min_height: proto.min_height.map(Into::into),
            max_height: proto.max_height.map(Into::into),
            position: proto.position.map(Into::into),
            z_index: proto.z_index,
            transform: proto.transform.map(Into::into),
            opacity: proto.opacity,
            overflow: proto
                .overflow
                .and_then(|o| proto::Overflow::try_from(o).ok())
                .map(Into::into),
            cursor: proto.cursor,
            responsive: proto.responsive.map(Into::into),
            flex: proto.flex,
            flex_grow: proto.flex_grow,
            flex_shrink: proto.flex_shrink,
            flex_basis: proto.flex_basis,
            align_self: proto.align_self,
            grid_column: proto.grid_column,
            grid_row: proto.grid_row,
            grid_area: proto.grid_area,
            justify_self: proto.justify_self,
            gap: proto.gap,
            color: proto.color,
            font_size: proto.font_size,
            font_weight: proto.font_weight,
            font_family: proto.font_family,
            line_height: proto.line_height,
            letter_spacing: proto.letter_spacing,
            text_align: proto.text_align,
            text_decoration: proto.text_decoration,
            text_transform: proto.text_transform,
            white_space: proto.white_space,
            word_break: proto.word_break,
            visibility: proto.visibility,
            user_select: proto.user_select,
            pointer_events: proto.pointer_events,
            transition: proto.transition,
            animation: proto.animation,
            display: proto.display,
            outline: proto.outline,
            outline_offset: proto.outline_offset,
            filter: proto.filter,
            backdrop_filter: proto.backdrop_filter,
            aspect_ratio: proto.aspect_ratio,
        }
    }
}

impl From<ResponsiveOverrides> for proto::ResponsiveOverrides {
    fn from(value: ResponsiveOverrides) -> Self {
        proto::ResponsiveOverrides {
            sm: value.sm.map(Into::into),
            md: value.md.map(Into::into),
            lg: value.lg.map(Into::into),
            xl: value.xl.map(Into::into),
            xxl: value.xxl.map(Into::into),
        }
    }
}

impl From<proto::ResponsiveOverrides> for ResponsiveOverrides {
    fn from(proto: proto::ResponsiveOverrides) -> Self {
        ResponsiveOverrides {
            sm: proto.sm.map(Into::into),
            md: proto.md.map(Into::into),
            lg: proto.lg.map(Into::into),
            xl: proto.xl.map(Into::into),
            xxl: proto.xxl.map(Into::into),
        }
    }
}

impl From<BreakpointStyle> for proto::BreakpointStyle {
    fn from(value: BreakpointStyle) -> Self {
        proto::BreakpointStyle {
            class_name: value.class_name,
            display: value.display,
            flex_direction: value.flex_direction,
            justify_content: value.justify_content,
            align_items: value.align_items,
            gap: value.gap,
            grid_cols: value.grid_cols,
            width: value.width.map(Into::into),
            height: value.height.map(Into::into),
            padding: value.padding.map(Into::into),
            margin: value.margin.map(Into::into),
            hidden: value.hidden,
            font_size: value.font_size,
            text_align: value.text_align,
            order: value.order,
        }
    }
}

impl From<proto::BreakpointStyle> for BreakpointStyle {
    fn from(proto: proto::BreakpointStyle) -> Self {
        BreakpointStyle {
            class_name: proto.class_name,
            display: proto.display,
            flex_direction: proto.flex_direction,
            justify_content: proto.justify_content,
            align_items: proto.align_items,
            gap: proto.gap,
            grid_cols: proto.grid_cols,
            width: proto.width.map(Into::into),
            height: proto.height.map(Into::into),
            padding: proto.padding.map(Into::into),
            margin: proto.margin.map(Into::into),
            hidden: proto.hidden,
            font_size: proto.font_size,
            text_align: proto.text_align,
            order: proto.order,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_default() {
        let size = Size::default();
        assert!(size.value.is_empty());
    }

    #[test]
    fn test_size_constructors() {
        let px = Size::px(100);
        assert_eq!(px.value, "100px");

        let percent = Size::percent(50);
        assert_eq!(percent.value, "50%");

        let auto = Size::auto();
        assert_eq!(auto.value, "auto");

        let full = Size::full();
        assert_eq!(full.value, "100%");
    }

    #[test]
    fn test_spacing_default() {
        let spacing = Spacing::default();
        assert!(spacing.top.is_none());
        assert!(spacing.right.is_none());
        assert!(spacing.bottom.is_none());
        assert!(spacing.left.is_none());
    }

    #[test]
    fn test_spacing_new() {
        let spacing = Spacing::new("16px");
        assert_eq!(spacing.top.as_deref(), Some("16px"));
        assert_eq!(spacing.right.as_deref(), Some("16px"));
        assert_eq!(spacing.bottom.as_deref(), Some("16px"));
        assert_eq!(spacing.left.as_deref(), Some("16px"));
    }

    #[test]
    fn test_spacing_from_str() {
        let spacing: Spacing = "8px".into();
        assert_eq!(spacing.top.as_deref(), Some("8px"));
        assert_eq!(spacing.left.as_deref(), Some("8px"));
    }

    #[test]
    fn test_overflow_default() {
        let overflow = Overflow::default();
        assert_eq!(overflow, Overflow::Visible);
    }

    #[test]
    fn test_overflow_variants() {
        assert!(matches!(Overflow::Hidden, Overflow::Hidden));
        assert!(matches!(Overflow::Scroll, Overflow::Scroll));
        assert!(matches!(Overflow::Auto, Overflow::Auto));
    }

    #[test]
    fn test_position_default() {
        let pos = Position::default();
        assert!(pos.top.is_none());
        assert!(pos.right.is_none());
        assert!(pos.bottom.is_none());
        assert!(pos.left.is_none());
    }

    #[test]
    fn test_position_absolute() {
        let pos = Position::absolute();
        assert_eq!(pos.position_type, "absolute");
    }

    #[test]
    fn test_position_relative() {
        let pos = Position::relative();
        assert_eq!(pos.position_type, "relative");
    }

    #[test]
    fn test_position_fixed() {
        let pos = Position::fixed();
        assert_eq!(pos.position_type, "fixed");
    }

    #[test]
    fn test_transform_default() {
        let transform = Transform::default();
        assert!(transform.translate.is_none());
        assert!(transform.rotate.is_none());
        assert!(transform.scale.is_none());
        assert!(transform.transform_origin.is_none());
        assert!(transform.skew.is_none());
    }

    #[test]
    fn test_gradient_creation() {
        let gradient = Gradient {
            gradient_type: "linear".to_string(),
            angle: Some(45.0),
            direction: Some("to right".to_string()),
            stops: vec![],
        };
        assert_eq!(gradient.gradient_type, "linear");
        assert_eq!(gradient.direction.as_deref(), Some("to right"));
        assert!(gradient.stops.is_empty());
    }

    #[test]
    fn test_gradient_with_stops() {
        let gradient = Gradient {
            gradient_type: "linear".to_string(),
            angle: Some(45.0),
            direction: Some("45deg".to_string()),
            stops: vec![
                GradientStop {
                    color: "#ff0000".to_string(),
                    position: Some(0.0),
                },
                GradientStop {
                    color: "#0000ff".to_string(),
                    position: Some(100.0),
                },
            ],
        };
        assert_eq!(gradient.stops.len(), 2);
        assert_eq!(gradient.stops[0].color, "#ff0000");
        assert_eq!(gradient.stops[1].position, Some(100.0));
    }

    #[test]
    fn test_frontend_json_contract_is_canonical() {
        let style: Style = serde_json::from_value(serde_json::json!({
            "className": "example",
            "background": {
                "gradient": {
                    "type": "linear",
                    "angle": 45,
                    "direction": "to right",
                    "stops": [
                        { "color": "#ff0000", "position": 0 },
                        { "color": "#0000ff", "position": 1 },
                        { "color": "#00ff00" }
                    ]
                }
            },
            "shadow": {
                "x": "0",
                "y": "4px",
                "blur": "12px",
                "spread": "0",
                "color": "rgba(0,0,0,0.25)",
                "inset": false
            },
            "padding": { "top": "1px", "right": "2px", "bottom": "3px", "left": "4px" },
            "margin": { "top": "8px", "bottom": "12px" },
            "width": "320px",
            "height": "50%",
            "minWidth": "100px",
            "maxWidth": "90vw",
            "minHeight": "20px",
            "maxHeight": "80vh",
            "position": { "type": "absolute", "top": "0", "left": "1rem" },
            "transform": {
                "translate": "1px, 2px",
                "rotate": 15,
                "scale": "1.2",
                "transformOrigin": "center",
                "skew": "2deg, 3deg"
            },
            "overflow": "hidden",
            "responsiveOverrides": {
                "md": {
                    "className": "md-card",
                    "display": "grid",
                    "flexDirection": "row",
                    "justifyContent": "center",
                    "alignItems": "stretch",
                    "gap": "1rem",
                    "gridCols": 3,
                    "width": "90%",
                    "height": "auto",
                    "padding": { "left": "2rem" },
                    "margin": { "top": "1rem" },
                    "hidden": false,
                    "fontSize": "1rem",
                    "textAlign": "center",
                    "order": 2
                }
            },
            "gap": "1rem",
            "flex": "1 1 auto",
            "flexGrow": 1,
            "flexShrink": 0,
            "flexBasis": "20rem",
            "alignSelf": "center",
            "gridColumn": "span 2",
            "gridRow": "1",
            "gridArea": "main",
            "justifySelf": "stretch",
            "color": "#123456",
            "fontSize": "1.25rem",
            "fontWeight": "600",
            "fontFamily": "Inter",
            "lineHeight": "1.5",
            "letterSpacing": "0.01em",
            "textAlign": "left",
            "textDecoration": "underline",
            "textTransform": "uppercase",
            "whiteSpace": "pre-wrap",
            "wordBreak": "break-word",
            "opacity": 0.8,
            "visibility": "visible",
            "cursor": "pointer",
            "userSelect": "none",
            "pointerEvents": "auto",
            "zIndex": 4,
            "transition": "opacity 100ms",
            "animation": "pulse 1s",
            "display": "grid",
            "outline": "1px solid red",
            "outlineOffset": "2px",
            "filter": "grayscale(1)",
            "backdropFilter": "blur(8px)",
            "aspectRatio": "16 / 9"
        }))
        .expect("frontend style JSON should deserialize");

        let json = serde_json::to_value(&style).expect("style should serialize");
        assert_eq!(json["background"]["gradient"]["type"], "linear");
        assert_eq!(json["background"]["gradient"]["angle"], 45.0);
        assert!(json["background"]["gradient"].get("gradientType").is_none());
        assert_eq!(json["background"]["gradient"]["stops"][1]["position"], 1.0);
        assert!(
            json["background"]["gradient"]["stops"][2]
                .get("position")
                .is_none()
        );
        assert_eq!(json["position"]["type"], "absolute");
        assert!(json["position"].get("positionType").is_none());
        assert!(json.get("responsiveOverrides").is_some());
        assert!(json.get("responsive").is_none());
        assert_eq!(json["padding"]["top"], "1px");
        assert!(json["padding"].get("value").is_none());
        assert_eq!(json["shadow"]["x"], "0");
        assert!(json["shadow"].get("boxShadows").is_none());
        assert_eq!(json["width"], "320px");
    }

    #[test]
    fn test_previous_rust_json_shapes_are_accepted_and_normalized() {
        let style: Style = serde_json::from_value(serde_json::json!({
            "background": {
                "gradient": {
                    "gradientType": "linear",
                    "direction": "to right",
                    "stops": [
                        { "color": "red", "position": 0.0 },
                        { "color": "blue", "position": 1.0 }
                    ]
                }
            },
            "shadow": {
                "boxShadows": ["inset 1px 2px 3px 4px rgba(0, 0, 0, 0.5)"],
                "textShadow": "1px 1px black"
            },
            "margin": { "value": "8px 16px" },
            "padding": "4px",
            "width": { "value": "20rem" },
            "position": { "positionType": "fixed", "right": "0" },
            "responsive": { "sm": { "hidden": true } }
        }))
        .expect("previous Rust JSON should remain readable");

        let json = serde_json::to_value(style).expect("legacy style should normalize");
        assert_eq!(json["background"]["gradient"]["type"], "linear");
        assert!(json["background"]["gradient"].get("gradientType").is_none());
        assert_eq!(
            json["background"]["gradient"]["stops"][1]["position"],
            100.0
        );
        assert_eq!(json["shadow"]["x"], "1px");
        assert_eq!(json["shadow"]["inset"], true);
        assert!(json["shadow"].get("boxShadows").is_none());
        assert_eq!(json["margin"]["top"], "8px");
        assert_eq!(json["margin"]["right"], "16px");
        assert_eq!(json["width"], "20rem");
        assert_eq!(json["position"]["type"], "fixed");
        assert!(json.get("responsiveOverrides").is_some());
        assert!(json.get("responsive").is_none());
    }

    #[test]
    fn test_json_schema_uses_frontend_wire_names_and_shapes() {
        let schema = serde_json::to_value(schemars::schema_for!(Style))
            .expect("style schema should serialize");
        let style_properties = schema["properties"]
            .as_object()
            .expect("Style schema should have properties");
        assert!(style_properties.contains_key("responsiveOverrides"));
        assert!(!style_properties.contains_key("responsive"));

        let definitions = schema["$defs"]
            .as_object()
            .expect("Style schema should have definitions");
        let gradient_properties = definitions["Gradient"]["properties"]
            .as_object()
            .expect("Gradient schema should have properties");
        assert!(gradient_properties.contains_key("type"));
        assert!(gradient_properties.contains_key("angle"));
        assert!(gradient_properties.contains_key("direction"));
        assert!(!gradient_properties.contains_key("gradientType"));

        let position_properties = definitions["Position"]["properties"]
            .as_object()
            .expect("Position schema should have properties");
        assert!(position_properties.contains_key("type"));
        assert!(!position_properties.contains_key("positionType"));

        let spacing_properties = definitions["Spacing"]["properties"]
            .as_object()
            .expect("Spacing schema should have properties");
        assert!(spacing_properties.contains_key("top"));
        assert!(spacing_properties.contains_key("right"));
        assert!(spacing_properties.contains_key("bottom"));
        assert!(spacing_properties.contains_key("left"));
        assert!(!spacing_properties.contains_key("value"));

        let shadow_properties = definitions["Shadow"]["properties"]
            .as_object()
            .expect("Shadow schema should have properties");
        for property in ["x", "y", "blur", "spread", "color", "inset"] {
            assert!(shadow_properties.contains_key(property));
        }
        assert!(!shadow_properties.contains_key("boxShadows"));

        assert_eq!(definitions["Size"]["type"], "string");
    }

    #[test]
    fn test_extended_style_proto_round_trip() {
        let style: Style = serde_json::from_value(serde_json::json!({
            "background": { "gradient": {
                "type": "conic", "angle": 30, "direction": "from 30deg",
                "stops": [{ "color": "red" }, { "color": "blue", "position": 100 }]
            }},
            "border": { "width": "1px", "style": "solid", "color": "red", "radius": "4px" },
            "shadow": { "x": "1px", "y": "2px", "blur": "3px", "spread": "4px", "color": "black", "inset": true },
            "padding": { "top": "1px", "right": "2px", "bottom": "3px", "left": "4px" },
            "margin": { "top": "5px" },
            "width": "320px", "height": "200px", "minWidth": "10px", "maxWidth": "90%",
            "minHeight": "20px", "maxHeight": "80vh",
            "position": { "type": "sticky", "top": "0" },
            "transform": { "translate": "1px,2px", "rotate": 10, "scale": "2", "transformOrigin": "center", "skew": "1deg" },
            "zIndex": 2, "opacity": 0.5, "overflow": "auto", "cursor": "pointer",
            "responsiveOverrides": { "lg": { "className": "card", "display": "grid", "flexDirection": "row", "justifyContent": "center", "alignItems": "center", "gap": "1rem", "gridCols": 2, "width": "80%", "height": "auto", "padding": { "left": "1rem" }, "margin": { "top": "2rem" }, "hidden": false, "fontSize": "2rem", "textAlign": "center", "order": 1 } },
            "flex": "1", "flexGrow": 1, "flexShrink": 0, "flexBasis": "auto", "alignSelf": "center",
            "gridColumn": "span 2", "gridRow": "1", "gridArea": "main", "justifySelf": "stretch", "gap": "1rem",
            "color": "red", "fontSize": "1rem", "fontWeight": "600", "fontFamily": "Inter", "lineHeight": "1.5", "letterSpacing": "1px", "textAlign": "left", "textDecoration": "none", "textTransform": "uppercase", "whiteSpace": "nowrap", "wordBreak": "break-word",
            "visibility": "visible", "userSelect": "none", "pointerEvents": "auto", "transition": "all 1s", "animation": "pulse 1s", "display": "grid", "outline": "none", "outlineOffset": "2px", "filter": "blur(1px)", "backdropFilter": "blur(2px)", "aspectRatio": "16 / 9"
        }))
        .expect("complete frontend style should deserialize");

        let before = serde_json::to_value(&style).expect("style should serialize");
        let restored = Style::from(proto::Style::from(style));
        let after = serde_json::to_value(&restored).expect("restored style should serialize");
        assert_eq!(after, before);
    }

    #[test]
    fn test_gradient_proto_position_unit_migration() {
        let legacy = proto::Gradient {
            gradient_type: "linear".to_string(),
            direction: None,
            stops: vec![
                proto::GradientStop {
                    color: "red".to_string(),
                    position: Some(0.0),
                },
                proto::GradientStop {
                    color: "blue".to_string(),
                    position: Some(1.0),
                },
            ],
            angle: None,
            stop_positions_are_percent: None,
        };
        let migrated = Gradient::from(legacy);
        assert_eq!(migrated.stops[0].position, Some(0.0));
        assert_eq!(migrated.stops[1].position, Some(100.0));

        let canonical = Gradient {
            gradient_type: "linear".to_string(),
            angle: None,
            direction: None,
            stops: vec![GradientStop {
                color: "blue".to_string(),
                position: Some(1.0),
            }],
        };
        let proto = proto::Gradient::from(canonical);
        assert_eq!(proto.stop_positions_are_percent, Some(true));
        let restored = Gradient::from(proto);
        assert_eq!(restored.stops[0].position, Some(1.0));
    }

    #[test]
    fn test_gradient_stop() {
        let stop = GradientStop {
            color: "#ff0000".to_string(),
            position: Some(50.0),
        };
        assert_eq!(stop.color, "#ff0000");
        assert_eq!(stop.position, Some(50.0));
    }

    #[test]
    fn test_background_color() {
        let bg = Background::Color("#ffffff".to_string());
        match bg {
            Background::Color(color) => assert_eq!(color, "#ffffff"),
            _ => panic!("Expected Color variant"),
        }
    }

    #[test]
    fn test_background_gradient() {
        let gradient = Gradient {
            gradient_type: "linear".to_string(),
            angle: None,
            direction: None,
            stops: vec![],
        };
        let bg = Background::Gradient(gradient);
        assert!(matches!(bg, Background::Gradient(_)));
    }

    #[test]
    fn test_background_blur() {
        let bg = Background::Blur("10px".to_string());
        match bg {
            Background::Blur(blur) => assert_eq!(blur, "10px"),
            _ => panic!("Expected Blur variant"),
        }
    }

    #[test]
    fn test_border_default() {
        let border = Border::default();
        assert!(border.width.is_none());
        assert!(border.style.is_none());
        assert!(border.color.is_none());
        assert!(border.radius.is_none());
    }

    #[test]
    fn test_border_with_values() {
        let border = Border {
            width: Some("2px".to_string()),
            style: Some("solid".to_string()),
            color: Some("#000000".to_string()),
            radius: Some("8px".to_string()),
        };
        assert_eq!(border.width.as_deref(), Some("2px"));
        assert_eq!(border.style.as_deref(), Some("solid"));
        assert_eq!(border.color.as_deref(), Some("#000000"));
        assert_eq!(border.radius.as_deref(), Some("8px"));
    }

    #[test]
    fn test_border_builder_methods() {
        let border = Border::new().with_width("1px").with_radius("4px");
        assert_eq!(border.width.as_deref(), Some("1px"));
        assert_eq!(border.radius.as_deref(), Some("4px"));
    }

    #[test]
    fn test_border_to_css() {
        let border = Border {
            width: Some("2px".to_string()),
            style: Some("solid".to_string()),
            color: Some("#000".to_string()),
            radius: Some("4px".to_string()),
        };
        let css = border.to_css();
        assert!(css.contains("border-width"));
        assert!(css.contains("border-style"));
    }

    #[test]
    fn test_shadow_default() {
        let shadow = Shadow::default();
        assert!(shadow.x.is_none());
        assert!(shadow.y.is_none());
        assert!(shadow.text_shadow.is_none());
    }

    #[test]
    fn test_shadow_with_values() {
        let shadow = Shadow {
            x: Some("0".to_string()),
            y: Some("2px".to_string()),
            blur: Some("4px".to_string()),
            color: Some("rgba(0,0,0,0.25)".to_string()),
            text_shadow: Some("1px 1px 2px black".to_string()),
            ..Shadow::default()
        };
        assert_eq!(shadow.x.as_deref(), Some("0"));
        assert!(shadow.text_shadow.is_some());
    }

    #[test]
    fn test_shadow_to_css() {
        let shadow = Shadow {
            x: Some("0".to_string()),
            y: Some("2px".to_string()),
            blur: Some("4px".to_string()),
            color: Some("rgba(0,0,0,0.25)".to_string()),
            ..Shadow::default()
        };
        let css = shadow.to_css();
        assert!(css.contains("box-shadow"));
    }

    #[test]
    fn test_style_default() {
        let style = Style::default();
        assert!(style.class_name.is_none());
        assert!(style.background.is_none());
        assert!(style.border.is_none());
        assert!(style.shadow.is_none());
        assert!(style.padding.is_none());
        assert!(style.margin.is_none());
        assert!(style.width.is_none());
        assert!(style.height.is_none());
        assert!(style.position.is_none());
        assert!(style.transform.is_none());
        assert!(style.opacity.is_none());
        assert!(style.overflow.is_none());
        assert!(style.cursor.is_none());
        assert!(style.responsive.is_none());
    }

    #[test]
    fn test_style_new() {
        let style = Style::new();
        assert!(style.class_name.is_none());
    }

    #[test]
    fn test_style_builder_methods() {
        let style = Style::new()
            .with_class("my-class")
            .with_padding("16px")
            .with_margin("8px")
            .with_width(Size::px(100))
            .with_height(Size::percent(50));

        assert_eq!(style.class_name.as_deref(), Some("my-class"));
        assert!(style.padding.is_some());
        assert!(style.margin.is_some());
        assert!(style.width.is_some());
        assert!(style.height.is_some());
    }

    #[test]
    fn test_style_to_tailwind_classes() {
        let style = Style::new().with_class("p-4 m-2 bg-blue-500");
        let classes = style.to_tailwind_classes();
        assert_eq!(classes, "p-4 m-2 bg-blue-500");
    }

    #[test]
    fn test_responsive_overrides_default() {
        let overrides = ResponsiveOverrides::default();
        assert!(overrides.sm.is_none());
        assert!(overrides.md.is_none());
        assert!(overrides.lg.is_none());
        assert!(overrides.xl.is_none());
        assert!(overrides.xxl.is_none());
    }

    #[test]
    fn test_breakpoint_style_default() {
        let bp = BreakpointStyle::default();
        assert!(bp.class_name.is_none());
        assert!(bp.display.is_none());
        assert!(bp.hidden.is_none());
    }

    #[test]
    fn test_responsive_overrides_with_breakpoints() {
        let overrides = ResponsiveOverrides {
            sm: Some(BreakpointStyle {
                hidden: Some(true),
                ..Default::default()
            }),
            md: Some(BreakpointStyle {
                hidden: Some(false),
                display: Some("flex".to_string()),
                ..Default::default()
            }),
            lg: None,
            xl: None,
            xxl: None,
        };

        assert!(overrides.sm.is_some());
        assert!(overrides.md.is_some());
        assert!(overrides.lg.is_none());

        if let Some(sm) = &overrides.sm {
            assert_eq!(sm.hidden, Some(true));
        }

        if let Some(md) = &overrides.md {
            assert_eq!(md.hidden, Some(false));
            assert_eq!(md.display.as_deref(), Some("flex"));
        }
    }

    #[test]
    fn test_transform_with_values() {
        let transform = Transform {
            translate: Some("10px 20px".to_string()),
            rotate: Some(45.0),
            scale: Some("1.5".to_string()),
            transform_origin: Some("center".to_string()),
            skew: Some("5deg".to_string()),
        };
        assert_eq!(transform.translate.as_deref(), Some("10px 20px"));
        assert_eq!(transform.rotate, Some(45.0));
        assert_eq!(transform.scale.as_deref(), Some("1.5"));
    }
}

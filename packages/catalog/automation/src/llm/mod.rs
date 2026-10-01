pub mod classify_screen;
pub mod extract_structured;
pub mod find_element;
pub mod heal;
pub mod heal_selector;
pub mod heal_template;
pub mod observe;
pub mod plan;
pub mod plan_actions;
pub mod rank_candidates;
pub mod resolve_element;

use crate::types::screen_frame::ScreenFrame;
use flow_like::flow::{node::Node, pin::PinOptions, variable::VariableType};
use flow_like_catalog_core::NodeImage;
use flow_like_types::json;

#[cfg(feature = "execute")]
use crate::types::screen_frame::{ModelImageBudget, fit_image_for_model};
#[cfg(feature = "execute")]
use flow_like::{bit::Bit, flow::execution::context::ExecutionContext};
#[cfg(feature = "execute")]
use flow_like_model_provider::history::{
    Content, ContentType, History, HistoryMessage, ImageUrl, MessageContent, Role,
};
#[cfg(feature = "execute")]
use flow_like_types::{Value, anyhow};

pub(crate) const COORDINATE_SPACE: &str = "desktop input coordinates (ready for the mouse nodes) when Frame is connected, otherwise pixels of the original screenshot";

/// Adds the screenshot inputs every vision node shares: a base64 string, an image that wins
/// when connected, and (for nodes that read or write coordinates) the screenshot's frame.
pub(crate) fn add_screenshot_pins(node: &mut Node, what: &str, with_frame: bool) {
    node.add_input_pin(
        "screenshot",
        "Screenshot",
        &format!(
            "{what} as base64 PNG, JPEG, WebP or GIF (a data URL is fine). Ignored when Image is connected"
        ),
        VariableType::String,
    )
    .set_default_value(Some(json::json!("")));

    node.add_input_pin(
        "image",
        "Image",
        &format!("{what} as an image, e.g. from the Screenshot node. Takes precedence over the base64 Screenshot"),
        VariableType::Struct,
    )
    .set_schema::<NodeImage>()
    .set_options(PinOptions::new().set_optional(true).build());

    if with_frame {
        node.add_input_pin(
            "frame",
            "Frame",
            &format!(
                "Screen frame of the screenshot, from the capture node. Coordinates are {COORDINATE_SPACE}"
            ),
            VariableType::Struct,
        )
        .set_schema::<ScreenFrame>()
        .set_options(PinOptions::new().set_optional(true).build());
    }
}

/// Cuts `text` to at most `max_bytes` bytes without splitting a UTF-8 character.
#[cfg(any(feature = "execute", test))]
pub(crate) fn truncate_on_char_boundary(text: &str, max_bytes: usize) -> &str {
    &text[..text.floor_char_boundary(max_bytes)]
}

/// A point written as `x,y`, `(x, y)` or `[x, y]`.
#[cfg(feature = "execute")]
pub(crate) fn parse_point(value: &str) -> Option<(f64, f64)> {
    let inner = value
        .trim()
        .trim_start_matches(['(', '['])
        .trim_end_matches([')', ']']);
    let (x, y) = inner.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Maps between the pixels of the image a model sees and the coordinates a node reports.
/// Its frame is the downscaled screenshot's frame: with a connected frame the "input" space is
/// the desktop; without one it is a synthetic frame whose input space is the original pixels.
#[cfg(any(feature = "execute", test))]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModelView {
    frame: ScreenFrame,
}

#[cfg(any(feature = "execute", test))]
impl ModelView {
    pub(crate) fn new(model_frame: ScreenFrame) -> Self {
        Self { frame: model_frame }
    }

    /// The full-size screenshot's frame: the connected one, checked against the image, or a
    /// synthetic identity frame so the mapping math is the same either way.
    pub(crate) fn source_frame(
        frame: Option<ScreenFrame>,
        image_size: (u32, u32),
    ) -> flow_like_types::Result<ScreenFrame> {
        let Some(frame) = frame else {
            return ScreenFrame::new(None, (0, 0, image_size.0, image_size.1), image_size);
        };
        frame.validate()?;
        if (frame.pixel_width, frame.pixel_height) != image_size {
            return Err(flow_like_types::anyhow!(
                "Frame describes a {}x{} screenshot but the image is {}x{}; connect the frame captured with this screenshot",
                frame.pixel_width,
                frame.pixel_height,
                image_size.0,
                image_size.1
            ));
        }
        Ok(frame)
    }

    pub(crate) fn size(&self) -> (u32, u32) {
        (self.frame.pixel_width, self.frame.pixel_height)
    }

    pub(crate) fn coordinate_hint(&self) -> String {
        let (width, height) = self.size();
        format!(
            "The screenshot is {width}x{height} pixels. Give every coordinate in pixels of this image, origin at its top-left corner."
        )
    }

    /// A point the model gave (pixels of its image) in the node's output space.
    pub(crate) fn point_from_model(&self, x: f64, y: f64) -> flow_like_types::Result<(i32, i32)> {
        self.frame.pixel_to_input(x, y)
    }

    pub(crate) fn width_from_model(&self, width: f64) -> Option<i32> {
        scale_length(width, self.frame.scale_x())
    }

    pub(crate) fn height_from_model(&self, height: f64) -> Option<i32> {
        scale_length(height, self.frame.scale_y())
    }

    /// A point in the node's output space as pixels of the model's image, `None` when off-image.
    pub(crate) fn point_to_model(&self, x: i32, y: i32) -> Option<(u32, u32)> {
        self.frame.input_to_pixel(x, y)
    }
}

#[cfg(any(feature = "execute", test))]
fn scale_length(length: f64, scale: f64) -> Option<i32> {
    let scaled = (length / scale).round();
    (scaled.is_finite() && scaled >= 0.0 && scaled <= i32::MAX as f64).then_some(scaled as i32)
}

#[cfg(feature = "execute")]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ModelImage {
    pub(crate) media_type: String,
    pub(crate) base64: String,
}

#[cfg(feature = "execute")]
impl ModelImage {
    fn content(&self) -> Content {
        Content::Image {
            content_type: ContentType::ImageUrl,
            image_url: ImageUrl {
                url: format!("data:{};base64,{}", self.media_type, self.base64),
                detail: None,
                media_type: Some(self.media_type.clone()),
                additional_params: None,
            },
        }
    }
}

#[cfg(feature = "execute")]
pub(crate) struct Screenshot {
    pub(crate) image: ModelImage,
    pub(crate) view: ModelView,
}

#[cfg(feature = "execute")]
pub(crate) enum ImageSource {
    Encoded(String),
    Decoded(image::DynamicImage),
}

/// Decodes a screenshot, downscales it to the model budget and pairs it with the frame of
/// the image the model will actually see.
#[cfg(feature = "execute")]
pub(crate) fn prepare_screenshot(
    source: ImageSource,
    frame: Option<ScreenFrame>,
) -> flow_like_types::Result<Screenshot> {
    let (image, passthrough) = match source {
        ImageSource::Encoded(text) => {
            let (image, media_type, payload) = decode_base64_image(&text)?;
            (image, media_type.map(|media_type| (media_type, payload)))
        }
        ImageSource::Decoded(image) => (image, None),
    };
    let source_frame = ModelView::source_frame(frame, (image.width(), image.height()))?;
    let (resized, model_frame) =
        fit_image_for_model(&image, &source_frame, ModelImageBudget::default())?;
    let unchanged = (resized.width(), resized.height()) == (image.width(), image.height());
    let image = match passthrough {
        Some((media_type, base64)) if unchanged => ModelImage {
            media_type: media_type.to_string(),
            base64,
        },
        _ => encode_png(&resized)?,
    };
    Ok(Screenshot {
        image,
        view: ModelView::new(model_frame),
    })
}

#[cfg(feature = "execute")]
fn decode_base64_image(
    text: &str,
) -> flow_like_types::Result<(image::DynamicImage, Option<&'static str>, String)> {
    use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};

    let trimmed = text.trim();
    let payload = trimmed
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(','))
        .map_or(trimmed, |(_, payload)| payload);
    let payload: String = payload
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    let bytes = STANDARD.decode(&payload).map_err(|e| {
        anyhow!(
            "Screenshot is not valid base64 ({} characters): {e}",
            payload.len()
        )
    })?;
    let format = image::guess_format(&bytes)
        .map_err(|e| anyhow!("Screenshot bytes are not a recognised image format: {e}"))?;
    let image = image::load_from_memory_with_format(&bytes, format)
        .map_err(|e| anyhow!("Could not decode the {format:?} screenshot: {e}"))?;
    let media_type = match format {
        image::ImageFormat::Png => Some("image/png"),
        image::ImageFormat::Jpeg => Some("image/jpeg"),
        image::ImageFormat::WebP => Some("image/webp"),
        image::ImageFormat::Gif => Some("image/gif"),
        _ => None,
    };
    Ok((image, media_type, payload))
}

#[cfg(feature = "execute")]
fn encode_png(image: &image::DynamicImage) -> flow_like_types::Result<ModelImage> {
    use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};

    let eight_bit;
    let image = match image {
        image::DynamicImage::ImageRgba8(_)
        | image::DynamicImage::ImageRgb8(_)
        | image::DynamicImage::ImageLuma8(_)
        | image::DynamicImage::ImageLumaA8(_) => image,
        other => {
            eight_bit = image::DynamicImage::ImageRgba8(other.to_rgba8());
            &eight_bit
        }
    };
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| {
            anyhow!(
                "Could not encode the {}x{} screenshot as PNG: {e}",
                image.width(),
                image.height()
            )
        })?;
    Ok(ModelImage {
        media_type: "image/png".to_string(),
        base64: STANDARD.encode(bytes),
    })
}

/// Reads an optional input: `None` when the pin is missing (older node instance), unconnected
/// without a value, or empty.
#[cfg(feature = "execute")]
pub(crate) async fn optional_pin<T: serde::de::DeserializeOwned>(
    context: &ExecutionContext,
    name: &str,
) -> flow_like_types::Result<Option<T>> {
    let Ok(pin) = context.get_pin_by_name(name).await else {
        return Ok(None);
    };
    let connected = !pin.depends_on().is_empty();
    let value: Value = match context.evaluate_pin_ref(pin).await {
        Ok(value) => value,
        Err(_) if !connected => return Ok(None),
        Err(error) => return Err(anyhow!("Could not read input '{name}': {error}")),
    };
    let empty = value.is_null()
        || value.as_object().is_some_and(|object| object.is_empty())
        || value.as_str().is_some_and(str::is_empty);
    if empty {
        return Ok(None);
    }
    json::from_value(value)
        .map(Some)
        .map_err(|e| anyhow!("Input '{name}' does not hold a valid value: {e}"))
}

/// Loads the screenshot from `image` (preferred) or the base64 `screenshot` pin, together with
/// the optional `frame`. `None` only when neither is set.
#[cfg(feature = "execute")]
pub(crate) async fn load_screenshot(
    context: &mut ExecutionContext,
) -> flow_like_types::Result<Option<Screenshot>> {
    let frame = optional_pin::<ScreenFrame>(context, "frame").await?;
    let source = match optional_pin::<NodeImage>(context, "image").await? {
        Some(node_image) => {
            let image = node_image.get_image(context).await?;
            let image = image.lock().await.clone();
            ImageSource::Decoded(image)
        }
        None => {
            let encoded: String = context.evaluate_pin("screenshot").await.unwrap_or_default();
            if encoded.trim().is_empty() {
                return Ok(None);
            }
            ImageSource::Encoded(encoded)
        }
    };
    let screenshot = tokio::task::spawn_blocking(move || prepare_screenshot(source, frame))
        .await
        .map_err(|e| anyhow!("Screenshot preparation task failed: {e}"))??;
    Ok(Some(screenshot))
}

#[cfg(feature = "execute")]
pub(crate) async fn require_screenshot(
    context: &mut ExecutionContext,
) -> flow_like_types::Result<Screenshot> {
    load_screenshot(context)
        .await?
        .ok_or_else(|| anyhow!("No screenshot: connect Image or provide a base64 Screenshot"))
}

/// The single user turn a vision node sends: every image, then the full instructions.
#[cfg(feature = "execute")]
pub(crate) fn vision_history(images: &[&ModelImage], instructions: &str) -> History {
    let mut parts: Vec<Content> = images.iter().map(|image| image.content()).collect();
    parts.push(Content::Text {
        content_type: ContentType::Text,
        text: instructions.to_string(),
    });
    History::new(
        String::new(),
        vec![HistoryMessage {
            role: Role::User,
            content: MessageContent::Contents(parts),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            annotations: None,
        }],
    )
}

/// The one tool a vision node forces the model to call; its arguments are the node's answer.
#[cfg(feature = "execute")]
pub(crate) struct SubmitTool {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) parameters: Value,
}

#[cfg(feature = "execute")]
impl rig::tool::Tool for SubmitTool {
    const NAME: &'static str = "submit";
    type Error = std::convert::Infallible;
    type Args = Value;
    type Output = Value;

    fn name(&self) -> String {
        self.name.to_string()
    }

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: self.name.to_string(),
            description: self.description.to_string(),
            parameters: self.parameters.clone(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(args)
    }
}

/// Sends `history` (images and instructions) with `tool` forced and returns the arguments of
/// the model's call to it, or `None` when the model answered without calling it. Models that
/// reject a forced tool choice are handled by the provider layer.
#[cfg(feature = "execute")]
pub(crate) async fn call_tool(
    context: &mut ExecutionContext,
    model: &Bit,
    history: History,
    preamble: &str,
    tool: SubmitTool,
) -> flow_like_types::Result<Option<Value>> {
    use rig::completion::Completion;

    let name = tool.name;
    let (prompt, chat_history) = history
        .extract_prompt_and_history()
        .map_err(|e| anyhow!("Could not convert the `{name}` request into model messages: {e}"))?;
    let agent = model
        .agent(context, &Some(history))
        .await?
        .preamble(preamble)
        .tool(tool)
        .tool_choice(rig::message::ToolChoice::Required)
        .build();
    let response = agent
        .completion(prompt, chat_history)
        .await
        .map_err(|e| anyhow!("Could not build the `{name}` model request: {e}"))?
        .send()
        .await
        .map_err(|e| anyhow!("Model request for `{name}` failed: {e}"))?;
    Ok(response
        .choice
        .into_iter()
        .filter_map(|content| match content {
            rig::message::AssistantContent::ToolCall(call) if call.function.name == name => {
                Some(call.function.arguments)
            }
            _ => None,
        })
        .last()
        .map(|arguments| match arguments {
            Value::String(raw) => json::from_str(&raw).unwrap_or(Value::String(raw)),
            other => other,
        }))
}

#[cfg(feature = "execute")]
pub(crate) fn missing_tool_call(name: &str) -> flow_like_types::Error {
    anyhow!(
        "The model answered without calling `{name}`; use a vision model that supports tool calling"
    )
}

#[cfg(feature = "execute")]
pub(crate) fn parse_tool_args<T: serde::de::DeserializeOwned>(
    name: &str,
    arguments: &Value,
) -> flow_like_types::Result<T> {
    T::deserialize(arguments).map_err(|e| {
        let raw = arguments.to_string();
        anyhow!(
            "The model's `{name}` arguments are malformed: {e}; received {}",
            truncate_on_char_boundary(&raw, 500)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        let text = "äöü€😀abc";
        assert_eq!(truncate_on_char_boundary(text, 1), "");
        assert_eq!(truncate_on_char_boundary(text, 2), "ä");
        assert_eq!(truncate_on_char_boundary(text, 7), "äöü");
        assert_eq!(truncate_on_char_boundary(text, 9), "äöü€");
        assert_eq!(truncate_on_char_boundary(text, 12), "äöü€");
        assert_eq!(truncate_on_char_boundary(text, 13), "äöü€😀");
        assert_eq!(truncate_on_char_boundary(text, 1000), text);
        let html = format!("<p>{}", "é".repeat(20_000));
        let cut = truncate_on_char_boundary(&html, 30_000);
        assert_eq!(cut.len(), 29_999);
        assert!(cut.ends_with('é'));
    }

    fn model_view(frame: Option<ScreenFrame>, image_size: (u32, u32)) -> ModelView {
        let source = ModelView::source_frame(frame, image_size).unwrap();
        let (width, height) =
            crate::types::screen_frame::ModelImageBudget::default().fit(image_size.0, image_size.1);
        ModelView::new(source.resized(width, height).unwrap())
    }

    #[test]
    fn retina_frame_maps_model_pixels_to_desktop_input() {
        let frame = ScreenFrame::new(Some(1), (-1440, -200, 1440, 900), (2880, 1800)).unwrap();
        let view = model_view(Some(frame), (2880, 1800));
        let (width, height) = view.size();
        assert!(width <= 1568);
        let (x, y) = view
            .point_from_model(width as f64 / 2.0, height as f64 / 2.0)
            .unwrap();
        assert!((x + 720).abs() <= 1 && (y - 250).abs() <= 1);
        assert_eq!(view.point_from_model(0.0, 0.0).unwrap(), (-1440, -200));
        assert!(view.point_from_model(width as f64, 0.0).is_err());
        assert!(view.point_from_model(-1.0, 10.0).is_err());
        assert!(view.point_from_model(f64::NAN, 10.0).is_err());
        let back = view.point_to_model(-720, 250).unwrap();
        assert!((back.0 as i64 - (width / 2) as i64).abs() <= 1);
        assert_eq!(view.point_to_model(0, 0), None);
        assert_eq!(view.width_from_model(width as f64), Some(1440));
    }

    #[test]
    fn missing_frame_maps_model_pixels_to_original_screenshot_pixels() {
        let view = model_view(None, (3136, 1960));
        assert_eq!(view.size(), (1356, 847));
        let (x, y) = view.point_from_model(678.0, 423.5).unwrap();
        assert!((x - 1568).abs() <= 2 && (y - 980).abs() <= 2);
        assert_eq!(view.height_from_model(-5.0), None);

        let small = model_view(None, (800, 600));
        assert_eq!(small.size(), (800, 600));
        assert_eq!(small.point_from_model(12.0, 34.0).unwrap(), (12, 34));
        assert_eq!(small.point_to_model(12, 34), Some((12, 34)));
        assert!(small.point_from_model(800.0, 10.0).is_err());
    }

    #[test]
    fn frame_that_does_not_match_the_image_is_rejected() {
        let frame = ScreenFrame::new(None, (0, 0, 1440, 900), (2880, 1800)).unwrap();
        let error = ModelView::source_frame(Some(frame), (1440, 900)).unwrap_err();
        assert!(error.to_string().contains("2880x1800"));
    }

    #[cfg(feature = "execute")]
    fn png_base64(width: u32, height: u32) -> String {
        use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::new(width, height));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        STANDARD.encode(bytes)
    }

    #[cfg(feature = "execute")]
    #[test]
    fn screenshots_are_downscaled_and_keep_their_frame() {
        let encoded = png_base64(3200, 100);
        let screenshot = prepare_screenshot(
            ImageSource::Encoded(format!("data:image/png;base64,{encoded}")),
            None,
        )
        .unwrap();
        assert_eq!(
            screenshot.view.size(),
            crate::types::screen_frame::ModelImageBudget::default().fit(3200, 100)
        );
        assert!(screenshot.view.size().0 <= 1568);
        assert_eq!(screenshot.image.media_type, "image/png");
        assert_ne!(screenshot.image.base64, encoded);
        let (x, _) = screenshot.view.point_from_model(784.0, 10.0).unwrap();
        assert!((x - 1600).abs() <= 2);

        let small = png_base64(40, 30);
        let screenshot = prepare_screenshot(ImageSource::Encoded(small.clone()), None).unwrap();
        assert_eq!(screenshot.image.base64, small);
        assert!(prepare_screenshot(ImageSource::Encoded("not an image".into()), None).is_err());
    }

    #[cfg(feature = "execute")]
    #[test]
    fn request_carries_the_image_and_the_full_instructions() {
        use rig::message::{DocumentSourceKind, Message, UserContent};

        let image = ModelImage {
            media_type: "image/png".to_string(),
            base64: png_base64(4, 4),
        };
        let instructions = "Find the UI element matching this description: \"the blue submit button\"\nContext: checkout page";
        let (prompt, history) = vision_history(&[&image], instructions)
            .extract_prompt_and_history()
            .unwrap();
        assert!(history.is_empty());
        let Message::User { content } = prompt else {
            panic!("prompt must be a user message");
        };
        let parts: Vec<UserContent> = content.into_iter().collect();
        assert_eq!(parts.len(), 2);
        let UserContent::Image(sent) = &parts[0] else {
            panic!("first part must be the screenshot, got {:?}", parts[0]);
        };
        assert!(matches!(&sent.data, DocumentSourceKind::Base64(data) if *data == image.base64));
        assert!(matches!(
            sent.media_type,
            Some(rig::message::ImageMediaType::PNG)
        ));
        let UserContent::Text(text) = &parts[1] else {
            panic!("second part must be the instructions, got {:?}", parts[1]);
        };
        assert_eq!(text.text, instructions);
    }

    #[cfg(feature = "execute")]
    #[test]
    fn tool_arguments_parse_with_context_on_failure() {
        #[derive(serde::Deserialize)]
        struct Args {
            found: bool,
        }
        let args: Args = parse_tool_args("t", &json::json!({"found": true})).unwrap();
        assert!(args.found);
        let error = parse_tool_args::<Args>("submit_x", &json::json!({"found": "yes"}))
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("submit_x") && error.contains("yes"));
    }
}

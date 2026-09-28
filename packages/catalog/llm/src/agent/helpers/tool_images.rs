//! Images returned by agent tools (flow functions and MCP servers).
//!
//! Tool results reach the model as text with `[image N]` placeholders; the images themselves
//! follow in one user message right after the tool results, because only some providers
//! accept images inside tool results while every vision provider accepts them in user turns.

use std::borrow::Cow;
use std::io::Cursor;

use flow_like::flow::execution::{LogLevel, context::ExecutionContext};
use flow_like_catalog_core::{NodeImage, NodeImageWrapper};
use flow_like_types::{
    Result, Value, anyhow,
    base64::{Engine as _, engine::general_purpose::STANDARD as BASE64},
    image::{self, DynamicImage, ImageFormat, codecs::jpeg::JpegEncoder, imageops::FilterType},
    json,
};
use rig::OneOrMany;
use rig::message::{ImageMediaType, Message, ToolResult, ToolResultContent, UserContent};

pub const MAX_IMAGES_PER_TOOL_RESULT: usize = 8;
pub const MAX_TOOL_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const TOOL_IMAGE_KEEP_RECENT: usize = 3;
pub const TOOL_IMAGE_PRUNE_SLACK: usize = 2;
pub const TOOL_IMAGE_MESSAGE_HEADER: &str = "Images returned by the tool calls above:";
pub const PRUNED_TOOL_IMAGE_STUB: &str = "[earlier screenshot omitted]";
const TEXT_ONLY_MODEL_PLACEHOLDER: &str = "[image omitted: the model does not accept image input]";
const JPEG_FALLBACK_MIN_PNG_BYTES: usize = 768 * 1024;
const JPEG_QUALITY: u8 = 85;
const MAX_DECODE_EDGE: u32 = 16_384;
const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;

/// Largest image a vision model should receive: long edge and total pixel budget.
#[derive(Clone, Copy, Debug)]
pub struct ModelImageBudget {
    pub max_long_edge: u32,
    pub max_pixels: u64,
}

impl Default for ModelImageBudget {
    fn default() -> Self {
        Self {
            max_long_edge: 1568,
            max_pixels: 1_150_000,
        }
    }
}

impl ModelImageBudget {
    pub fn fit(&self, width: u32, height: u32) -> (u32, u32) {
        if width == 0 || height == 0 {
            return (width, height);
        }
        let long_edge = width.max(height) as f64;
        let edge_scale = (self.max_long_edge as f64 / long_edge).min(1.0);
        let area_scale = (self.max_pixels as f64 / (width as f64 * height as f64))
            .sqrt()
            .min(1.0);
        let scale = edge_scale.min(area_scale);
        (
            ((width as f64 * scale).floor() as u32).max(1),
            ((height as f64 * scale).floor() as u32).max(1),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolResultOrigin {
    FlowFunction,
    Mcp,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolImageSource {
    NodeImage(String),
    Base64 { mime_type: String, data: String },
}

/// An image found in a tool result, addressed by the JSON pointer it will be replaced at.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageCandidate {
    pub pointer: String,
    pub source: ToolImageSource,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelImage {
    pub media_type: ImageMediaType,
    pub data: String,
    pub width: u32,
    pub height: u32,
}

impl ModelImage {
    fn user_content(&self) -> UserContent {
        UserContent::image_base64(self.data.clone(), Some(self.media_type.clone()), None)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexedImage {
    pub index: usize,
    pub image: ModelImage,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CandidateOutcome {
    Image(ModelImage),
    Omitted(String),
    Untouched,
}

/// A finished tool call: `text` is what the model and the chat history receive in the tool
/// result; `images` follow in the image message.
#[derive(Clone, Debug)]
pub struct ToolCallOutcome {
    pub id: String,
    pub call_id: Option<String>,
    pub name: String,
    pub arguments: Value,
    pub text: String,
    pub images: Vec<IndexedImage>,
}

impl ToolCallOutcome {
    pub fn new(
        id: String,
        call_id: Option<String>,
        name: String,
        arguments: Value,
        output: &Value,
        images: Vec<IndexedImage>,
    ) -> Self {
        let text = tool_result_text(output, &images);
        Self {
            id,
            call_id,
            name,
            arguments,
            text,
            images,
        }
    }
}

pub fn collect_image_candidates(value: &Value, origin: ToolResultOrigin) -> Vec<ImageCandidate> {
    let mut candidates = Vec::new();
    if origin == ToolResultOrigin::Mcp {
        collect_mcp_blocks(value, &mut candidates);
    }
    walk_value(
        value,
        &mut String::new(),
        origin == ToolResultOrigin::FlowFunction,
        &mut candidates,
    );
    candidates
}

fn collect_mcp_blocks(value: &Value, candidates: &mut Vec<ImageCandidate>) {
    let Some(blocks) = value.get("content").and_then(Value::as_array) else {
        return;
    };
    for (index, block) in blocks.iter().enumerate() {
        let (pointer, data, mime_type) = match block.get("type").and_then(Value::as_str) {
            Some("image") => (
                format!("/content/{index}/data"),
                block.get("data"),
                block.get("mimeType"),
            ),
            Some("resource") => {
                let resource = block.get("resource");
                (
                    format!("/content/{index}/resource/blob"),
                    resource.and_then(|resource| resource.get("blob")),
                    resource.and_then(|resource| resource.get("mimeType")),
                )
            }
            _ => continue,
        };
        let (Some(data), Some(mime_type)) = (
            data.and_then(Value::as_str),
            mime_type.and_then(Value::as_str),
        ) else {
            continue;
        };
        if let Some(source) = base64_image_source(mime_type, data) {
            candidates.push(ImageCandidate { pointer, source });
        }
    }
}

fn walk_value(
    value: &Value,
    pointer: &mut String,
    node_images: bool,
    candidates: &mut Vec<ImageCandidate>,
) {
    match value {
        Value::String(text) => {
            if let Some(source) = parse_image_data_url(text)
                && !candidates.iter().any(|candidate| candidate.pointer == *pointer)
            {
                candidates.push(ImageCandidate {
                    pointer: pointer.clone(),
                    source,
                });
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                walk_child(item, &index.to_string(), pointer, node_images, candidates);
            }
        }
        Value::Object(map) => {
            if node_images && collect_node_image(map, pointer, candidates) {
                return;
            }
            for (key, item) in map {
                let token = key.replace('~', "~0").replace('/', "~1");
                walk_child(item, &token, pointer, node_images, candidates);
            }
        }
        _ => {}
    }
}

fn walk_child(
    value: &Value,
    token: &str,
    pointer: &mut String,
    node_images: bool,
    candidates: &mut Vec<ImageCandidate>,
) {
    let parent_len = pointer.len();
    pointer.push('/');
    pointer.push_str(token);
    walk_value(value, pointer, node_images, candidates);
    pointer.truncate(parent_len);
}

/// Records a `NodeImage` (`{"image_ref": ...}`). Returns true when the whole object is the
/// image, so there is nothing else to walk.
fn collect_node_image(
    map: &json::Map<String, Value>,
    pointer: &str,
    candidates: &mut Vec<ImageCandidate>,
) -> bool {
    let Some(Value::String(image_ref)) = map.get("image_ref") else {
        return false;
    };
    let whole_object = map.len() == 1;
    candidates.push(ImageCandidate {
        pointer: if whole_object {
            pointer.to_string()
        } else {
            format!("{pointer}/image_ref")
        },
        source: ToolImageSource::NodeImage(image_ref.clone()),
    });
    whole_object
}

/// Parses a base64 raster `data:image/...` URL. SVG is text and stays in the tool result.
pub fn parse_image_data_url(text: &str) -> Option<ToolImageSource> {
    const PREFIX: &[u8] = b"data:image/";
    if text.len() <= PREFIX.len() || !text.as_bytes()[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
    {
        return None;
    }
    let (metadata, payload) = text["data:".len()..].split_once(',')?;
    let mut parts = metadata.split(';');
    let mime_type = parts.next()?.trim();
    if !parts.any(|part| part.trim().eq_ignore_ascii_case("base64")) {
        return None;
    }
    base64_image_source(mime_type, payload)
}

fn base64_image_source(mime_type: &str, data: &str) -> Option<ToolImageSource> {
    if let Some(source) = parse_image_data_url(data) {
        return Some(source);
    }
    let mime_type = mime_type.trim().to_ascii_lowercase();
    if !mime_type.starts_with("image/") || mime_type == "image/svg+xml" || data.is_empty() {
        return None;
    }
    Some(ToolImageSource::Base64 {
        mime_type,
        data: data.to_string(),
    })
}

/// Replaces every resolved candidate with its placeholder and numbers the delivered images
/// starting at `next_index`. Candidates without an outcome exceeded the per-result limit.
pub fn finish_extraction(
    mut value: Value,
    candidates: &[ImageCandidate],
    outcomes: Vec<CandidateOutcome>,
    next_index: &mut usize,
) -> (Value, Vec<IndexedImage>) {
    let mut outcomes = outcomes.into_iter();
    let mut images = Vec::new();
    for candidate in candidates {
        let placeholder = match outcomes.next() {
            Some(CandidateOutcome::Untouched) => continue,
            Some(CandidateOutcome::Omitted(reason)) => reason,
            Some(CandidateOutcome::Image(image)) => {
                let index = *next_index;
                *next_index += 1;
                images.push(IndexedImage { index, image });
                format!("[image {index}]")
            }
            None => format!(
                "[image omitted: a tool result may carry at most {MAX_IMAGES_PER_TOOL_RESULT} images]"
            ),
        };
        if let Some(slot) = value.pointer_mut(&candidate.pointer) {
            *slot = Value::String(placeholder);
        }
    }
    (value, images)
}

/// Pulls images out of a tool result so they can be sent to the model as images. Results
/// without images are returned unchanged.
pub async fn extract_tool_images(
    context: &mut ExecutionContext,
    value: Value,
    origin: ToolResultOrigin,
    deliver_images: bool,
    next_index: &mut usize,
) -> (Value, Vec<IndexedImage>) {
    let candidates = collect_image_candidates(&value, origin);
    if candidates.is_empty() {
        return (value, Vec::new());
    }
    let mut outcomes = Vec::with_capacity(candidates.len().min(MAX_IMAGES_PER_TOOL_RESULT));
    for candidate in candidates.iter().take(MAX_IMAGES_PER_TOOL_RESULT) {
        let outcome = resolve_candidate(context, &candidate.source, deliver_images).await;
        if let CandidateOutcome::Omitted(reason) = &outcome {
            context.log_message(
                &format!(
                    "Tool result image at '{}' was not sent to the model: {}",
                    candidate.pointer, reason
                ),
                LogLevel::Warn,
            );
        }
        outcomes.push(outcome);
    }
    finish_extraction(value, &candidates, outcomes, next_index)
}

async fn resolve_candidate(
    context: &mut ExecutionContext,
    source: &ToolImageSource,
    deliver_images: bool,
) -> CandidateOutcome {
    let prepared = match source {
        ToolImageSource::NodeImage(image_ref) => {
            if !deliver_images {
                return if node_image_exists(context, image_ref).await {
                    CandidateOutcome::Omitted(TEXT_ONLY_MODEL_PLACEHOLDER.to_string())
                } else {
                    CandidateOutcome::Untouched
                };
            }
            let node_image = NodeImage {
                image_ref: image_ref.clone(),
            };
            let Ok(image) = node_image.get_image(context).await else {
                return CandidateOutcome::Untouched;
            };
            let image = image.lock().await.clone();
            tokio::task::spawn_blocking(move || {
                prepare_model_image(image, ModelImageBudget::default())
            })
            .await
        }
        ToolImageSource::Base64 { mime_type, data } => {
            if !deliver_images {
                return CandidateOutcome::Omitted(TEXT_ONLY_MODEL_PLACEHOLDER.to_string());
            }
            let (mime_type, data) = (mime_type.clone(), data.clone());
            tokio::task::spawn_blocking(move || {
                decode_base64_image(&mime_type, &data)
                    .and_then(|image| prepare_model_image(image, ModelImageBudget::default()))
            })
            .await
        }
    };
    match prepared {
        Ok(Ok(image)) => CandidateOutcome::Image(image),
        Ok(Err(error)) => CandidateOutcome::Omitted(format!("[image omitted: {error}]")),
        Err(error) => CandidateOutcome::Omitted(format!(
            "[image omitted: image processing task failed: {error}]"
        )),
    }
}

async fn node_image_exists(context: &ExecutionContext, image_ref: &str) -> bool {
    context
        .cache
        .read()
        .await
        .get(image_ref)
        .is_some_and(|entry| entry.as_any().is::<NodeImageWrapper>())
}

pub fn decode_base64_image(mime_type: &str, data: &str) -> Result<DynamicImage> {
    let compact: Cow<str> = if data.bytes().any(|byte| byte.is_ascii_whitespace()) {
        Cow::Owned(data.chars().filter(|ch| !ch.is_ascii_whitespace()).collect())
    } else {
        Cow::Borrowed(data)
    };
    let estimated_bytes = compact.len() / 4 * 3;
    if estimated_bytes > MAX_TOOL_IMAGE_BYTES {
        return Err(anyhow!(
            "{mime_type} image is about {estimated_bytes} bytes, above the {MAX_TOOL_IMAGE_BYTES} byte limit"
        ));
    }
    let bytes = BASE64
        .decode(compact.as_bytes())
        .map_err(|error| anyhow!("{mime_type} image is not valid base64: {error}"))?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| anyhow!("Failed to read {mime_type} image header: {error}"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODE_EDGE);
    limits.max_image_height = Some(MAX_DECODE_EDGE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|error| anyhow!("Failed to decode {mime_type} image: {error}"))
}

/// Downscales to the model budget and re-encodes: PNG keeps text crisp; large opaque images
/// (photos) fall back to JPEG when that is smaller.
pub fn prepare_model_image(image: DynamicImage, budget: ModelImageBudget) -> Result<ModelImage> {
    let (width, height) = budget.fit(image.width(), image.height());
    if width == 0 || height == 0 {
        return Err(anyhow!("image has no pixels ({width}x{height})"));
    }
    let resized = if (width, height) == (image.width(), image.height()) {
        image
    } else {
        image.resize_exact(width, height, FilterType::Triangle)
    };
    let normalized = normalize_pixels(resized);

    let mut png = Vec::new();
    normalized
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|error| anyhow!("Failed to encode {width}x{height} tool image as PNG: {error}"))?;

    let (media_type, bytes) = match smaller_jpeg(&normalized, png.len())? {
        Some(jpeg) => (ImageMediaType::JPEG, jpeg),
        None => (ImageMediaType::PNG, png),
    };
    Ok(ModelImage {
        media_type,
        data: BASE64.encode(bytes),
        width,
        height,
    })
}

fn smaller_jpeg(image: &DynamicImage, png_len: usize) -> Result<Option<Vec<u8>>> {
    let DynamicImage::ImageRgb8(rgb) = image else {
        return Ok(None);
    };
    if png_len <= JPEG_FALLBACK_MIN_PNG_BYTES {
        return Ok(None);
    }
    let mut jpeg = Vec::new();
    rgb.write_with_encoder(JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY))
        .map_err(|error| {
            anyhow!(
                "Failed to encode {}x{} tool image as JPEG: {error}",
                rgb.width(),
                rgb.height()
            )
        })?;
    Ok((jpeg.len() < png_len).then_some(jpeg))
}

fn normalize_pixels(image: DynamicImage) -> DynamicImage {
    if !image.color().has_alpha() {
        return DynamicImage::ImageRgb8(image.to_rgb8());
    }
    let rgba = image.to_rgba8();
    if rgba.pixels().all(|pixel| pixel.0[3] == u8::MAX) {
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(rgba).to_rgb8())
    } else {
        DynamicImage::ImageRgba8(rgba)
    }
}

pub fn tool_result_text(output: &Value, images: &[IndexedImage]) -> String {
    let text = match output.as_str() {
        Some(text) => text.to_string(),
        None => json::to_string(output).unwrap_or_default(),
    };
    if images.is_empty() {
        return text;
    }
    let labels = images
        .iter()
        .map(|image| format!("[image {}]", image.index))
        .collect::<Vec<_>>()
        .join(", ");
    let noun = if images.len() == 1 { "Image" } else { "Images" };
    format!("{text}\n\n({noun} attached in the next message: {labels})")
}

/// Messages to append after the assistant's tool calls: the tool results first (providers
/// require them to follow the calls directly), then one user message with the images.
pub fn tool_turn_messages(outcomes: &[ToolCallOutcome]) -> Vec<Message> {
    let results = outcomes
        .iter()
        .map(|outcome| {
            UserContent::ToolResult(ToolResult {
                id: outcome.id.clone(),
                call_id: outcome.call_id.clone().or_else(|| Some(outcome.id.clone())),
                content: OneOrMany::one(ToolResultContent::text(outcome.text.clone())),
            })
        })
        .collect::<Vec<_>>();
    let mut messages = Vec::with_capacity(2);
    if let Ok(content) = OneOrMany::many(results) {
        messages.push(Message::User { content });
    }
    messages.extend(tool_image_message(outcomes));
    messages
}

fn tool_image_message(outcomes: &[ToolCallOutcome]) -> Option<Message> {
    let mut content = vec![UserContent::text(TOOL_IMAGE_MESSAGE_HEADER)];
    for outcome in outcomes {
        for image in &outcome.images {
            content.push(UserContent::text(format!(
                "[image {}] from `{}` (call {}):",
                image.index, outcome.name, outcome.id
            )));
            content.push(image.image.user_content());
        }
    }
    if content.len() == 1 {
        return None;
    }
    OneOrMany::many(content)
        .ok()
        .map(|content| Message::User { content })
}

pub fn is_tool_image_message(message: &Message) -> bool {
    matches!(
        message,
        Message::User { content }
            if matches!(content.first_ref(), UserContent::Text(text) if text.text == TOOL_IMAGE_MESSAGE_HEADER)
    )
}

/// Keeps the newest `keep` tool images in the in-flight history and turns older ones into
/// text stubs. Pruning waits until `keep + slack` images accumulate so the cached prompt
/// prefix changes in batches instead of on every step. Images of the newest image message
/// have not been seen by the model yet and are never pruned. Returns the number pruned.
pub fn prune_tool_images(history: &mut [Message], keep: usize, slack: usize) -> usize {
    let positions = history
        .iter()
        .enumerate()
        .filter(|(_, message)| is_tool_image_message(message))
        .flat_map(|(message_index, message)| {
            let Message::User { content } = message else {
                return Vec::new();
            };
            content
                .iter()
                .enumerate()
                .filter(|(_, part)| matches!(part, UserContent::Image(_)))
                .map(|(part_index, _)| (message_index, part_index))
                .collect()
        })
        .collect::<Vec<_>>();
    if positions.len() <= keep + slack {
        return 0;
    }
    let newest_message = positions.last().map(|(message_index, _)| *message_index);
    let unseen = positions
        .iter()
        .filter(|(message_index, _)| Some(*message_index) == newest_message)
        .count();
    let prune_count = positions.len() - keep.max(unseen);
    for (message_index, part_index) in positions.iter().take(prune_count) {
        if let Message::User { content } = &mut history[*message_index]
            && let Some(part) = content.iter_mut().nth(*part_index)
        {
            *part = UserContent::text(PRUNED_TOOL_IMAGE_STUB);
        }
    }
    prune_count
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_model_provider::history::{Content, HistoryMessage, MessageContent, Role};
    use flow_like_types::image::{Rgb, RgbImage, Rgba, RgbaImage};

    fn png_base64(width: u32, height: u32) -> String {
        let mut bytes = Vec::new();
        DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb([10, 20, 30])))
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        BASE64.encode(bytes)
    }

    fn model_image(tag: &str) -> ModelImage {
        ModelImage {
            media_type: ImageMediaType::PNG,
            data: tag.to_string(),
            width: 1,
            height: 1,
        }
    }

    fn sorted_pointers(candidates: &[ImageCandidate]) -> Vec<&str> {
        let mut pointers = candidates
            .iter()
            .map(|candidate| candidate.pointer.as_str())
            .collect::<Vec<_>>();
        pointers.sort_unstable();
        pointers
    }

    fn user_text(message: &Message) -> Vec<String> {
        let Message::User { content } = message else {
            panic!("expected a user message");
        };
        content
            .iter()
            .map(|part| match part {
                UserContent::Text(text) => text.text.clone(),
                UserContent::Image(image) => format!("<image {:?}>", image.data),
                UserContent::ToolResult(result) => match result.content.first_ref() {
                    ToolResultContent::Text(text) => format!("<result {}: {}>", result.id, text.text),
                    ToolResultContent::Image(_) => "<result image>".to_string(),
                },
                _ => "<other>".to_string(),
            })
            .collect()
    }

    fn image_message(images: usize) -> Message {
        let tags = (1..=images).map(|index| (index, "img")).collect::<Vec<_>>();
        tool_image_message(&[outcome("call", "screenshot", json::json!("ok"), &tags)]).unwrap()
    }

    fn count_images(history: &[Message]) -> usize {
        history
            .iter()
            .map(|message| match message {
                Message::User { content } => content
                    .iter()
                    .filter(|part| matches!(part, UserContent::Image(_)))
                    .count(),
                _ => 0,
            })
            .sum()
    }

    #[test]
    fn walker_finds_node_images_and_data_urls_at_any_depth() {
        let url = format!("data:image/png;base64,{}", png_base64(2, 2));
        let value = json::json!({
            "shots": [{ "image_ref": "a" }, { "meta": { "image_ref": "b", "width": 3 } }],
            "thumb": url,
            "a/b": ["DATA:IMAGE/JPEG;BASE64,AAAA"],
            "svg": "data:image/svg+xml;base64,PHN2Zz4=",
            "plain": "data:text/plain;base64,aGk=",
            "not_base64": "data:image/png,raw",
            "text": "see data:image/png;base64,AAAA",
        });
        let candidates = collect_image_candidates(&value, ToolResultOrigin::FlowFunction);
        assert_eq!(
            sorted_pointers(&candidates),
            vec!["/a~1b/0", "/shots/0", "/shots/1/meta/image_ref", "/thumb"]
        );
        let jpeg = candidates
            .iter()
            .find(|candidate| candidate.pointer == "/a~1b/0")
            .unwrap();
        assert_eq!(
            jpeg.source,
            ToolImageSource::Base64 {
                mime_type: "image/jpeg".into(),
                data: "AAAA".into()
            }
        );
        let node = candidates
            .iter()
            .find(|candidate| candidate.pointer == "/shots/0")
            .unwrap();
        assert_eq!(node.source, ToolImageSource::NodeImage("a".into()));
    }

    #[test]
    fn walker_handles_root_values_and_ignores_node_refs_from_mcp() {
        let root_ref = json::json!({ "image_ref": "abc" });
        let candidates = collect_image_candidates(&root_ref, ToolResultOrigin::FlowFunction);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].pointer, "");
        assert!(collect_image_candidates(&root_ref, ToolResultOrigin::Mcp).is_empty());

        let root_url = json::json!("data:image/gif;base64,R0lGOD");
        let candidates = collect_image_candidates(&root_url, ToolResultOrigin::FlowFunction);
        assert_eq!(candidates[0].pointer, "");

        let plain = json::json!({ "rows": [1, 2, { "name": "x" }], "ok": true });
        assert!(collect_image_candidates(&plain, ToolResultOrigin::FlowFunction).is_empty());
    }

    #[test]
    fn placeholders_replace_candidates_and_number_across_results() {
        let value = json::json!({
            "first": { "image_ref": "a" },
            "list": ["data:image/png;base64,AAAA", { "image_ref": "missing" }],
        });
        let mut candidates = collect_image_candidates(&value, ToolResultOrigin::FlowFunction);
        candidates.sort_by(|left, right| left.pointer.cmp(&right.pointer));
        let outcomes = vec![
            CandidateOutcome::Image(model_image("a")),
            CandidateOutcome::Image(model_image("b")),
            CandidateOutcome::Untouched,
        ];
        let mut next_index = 4;
        let (value, images) = finish_extraction(value, &candidates, outcomes, &mut next_index);
        assert_eq!(
            value,
            json::json!({
                "first": "[image 4]",
                "list": ["[image 5]", { "image_ref": "missing" }],
            })
        );
        assert_eq!(
            images.iter().map(|image| image.index).collect::<Vec<_>>(),
            vec![4, 5]
        );
        assert_eq!(next_index, 6);
    }

    #[test]
    fn images_beyond_the_per_result_limit_are_omitted_not_inlined() {
        let value = Value::Array(
            (0..MAX_IMAGES_PER_TOOL_RESULT + 2)
                .map(|_| json::json!("data:image/png;base64,AAAA"))
                .collect(),
        );
        let candidates = collect_image_candidates(&value, ToolResultOrigin::FlowFunction);
        let outcomes = (0..MAX_IMAGES_PER_TOOL_RESULT)
            .map(|_| CandidateOutcome::Image(model_image("x")))
            .collect();
        let mut next_index = 1;
        let (value, images) = finish_extraction(value, &candidates, outcomes, &mut next_index);
        assert_eq!(images.len(), MAX_IMAGES_PER_TOOL_RESULT);
        let items = value.as_array().unwrap();
        assert_eq!(items[0], json::json!("[image 1]"));
        let overflow = items[MAX_IMAGES_PER_TOOL_RESULT].as_str().unwrap();
        assert!(overflow.starts_with("[image omitted"));
        assert!(!json::to_string(&value).unwrap().contains("base64"));
    }

    #[test]
    fn mcp_image_blocks_become_images_and_text_blocks_stay_text() {
        use rmcp::model::{CallToolResult, Content as McpContent, ResourceContents};

        let data = png_base64(4, 4);
        let result = CallToolResult::success(vec![
            McpContent::text("Captured the page"),
            McpContent::image(data.clone(), "image/png"),
            McpContent::resource(ResourceContents::BlobResourceContents {
                uri: "file:///shot.jpg".into(),
                mime_type: Some("image/jpeg".into()),
                blob: "AAAA".into(),
                meta: None,
            }),
            McpContent::resource(ResourceContents::BlobResourceContents {
                uri: "file:///doc.pdf".into(),
                mime_type: Some("application/pdf".into()),
                blob: "JVBERi0=".into(),
                meta: None,
            }),
        ]);
        let value = json::to_value(result).unwrap();
        let candidates = collect_image_candidates(&value, ToolResultOrigin::Mcp);
        assert_eq!(
            sorted_pointers(&candidates),
            vec!["/content/1/data", "/content/2/resource/blob"]
        );
        assert_eq!(
            candidates[0].source,
            ToolImageSource::Base64 {
                mime_type: "image/png".into(),
                data: data.clone()
            }
        );

        let outcomes = vec![
            CandidateOutcome::Image(model_image("png")),
            CandidateOutcome::Omitted("[image omitted: bad]".into()),
        ];
        let mut next_index = 1;
        let (value, images) = finish_extraction(value, &candidates, outcomes, &mut next_index);
        assert_eq!(images.len(), 1);
        assert_eq!(value["content"][0]["text"], "Captured the page");
        assert_eq!(value["content"][1]["data"], "[image 1]");
        assert_eq!(value["content"][2]["resource"]["blob"], "[image omitted: bad]");
        assert_eq!(value["content"][3]["resource"]["blob"], "JVBERi0=");
        assert!(json::to_string(&value).unwrap().len() < 1024);
    }

    #[test]
    fn budget_limits_long_edge_and_area() {
        let budget = ModelImageBudget::default();
        let (width, height) = budget.fit(2880, 1800);
        assert!(width <= 1568 && (width as u64 * height as u64) <= 1_150_000);
        assert_eq!(budget.fit(800, 600), (800, 600));
        assert_eq!(budget.fit(0, 10), (0, 10));
        let (width, height) = budget.fit(10_000, 10);
        assert!((1567..=1568).contains(&width));
        assert_eq!(height, 1);
    }

    #[test]
    fn prepared_images_fit_the_budget_and_prefer_png() {
        let screenshot = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            3000,
            2000,
            Rgba([200, 200, 200, 255]),
        ));
        let prepared = prepare_model_image(screenshot, ModelImageBudget::default()).unwrap();
        assert_eq!(prepared.media_type, ImageMediaType::PNG);
        assert!(prepared.width <= 1568);
        assert!(prepared.width as u64 * prepared.height as u64 <= 1_150_000);
        let decoded = image::load_from_memory(&BASE64.decode(&prepared.data).unwrap()).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (prepared.width, prepared.height));
        assert!(!decoded.color().has_alpha());

        let transparent =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(20, 10, Rgba([0, 0, 0, 0])));
        let prepared = prepare_model_image(transparent, ModelImageBudget::default()).unwrap();
        let decoded = image::load_from_memory(&BASE64.decode(&prepared.data).unwrap()).unwrap();
        assert!(decoded.color().has_alpha());
    }

    #[test]
    fn large_photos_are_reencoded_as_jpeg() {
        let mut state = 0x2545_f491_u32;
        let noise = RgbImage::from_fn(1400, 800, |_, _| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let [r, g, b, _] = state.to_le_bytes();
            Rgb([r, g, b])
        });
        let prepared =
            prepare_model_image(DynamicImage::ImageRgb8(noise), ModelImageBudget::default())
                .unwrap();
        assert_eq!(prepared.media_type, ImageMediaType::JPEG);
    }

    #[test]
    fn base64_decoding_validates_size_and_payload() {
        let decoded = decode_base64_image("image/png", &png_base64(3, 2)).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (3, 2));
        let wrapped = png_base64(3, 2)
            .as_bytes()
            .chunks(10)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(decode_base64_image("image/png", &wrapped).is_ok());
        assert!(decode_base64_image("image/png", "not base64!").is_err());
        assert!(decode_base64_image("image/png", "aGVsbG8=").is_err());
        let oversized = "A".repeat(MAX_TOOL_IMAGE_BYTES / 3 * 4 + 8);
        let error = decode_base64_image("image/png", &oversized).unwrap_err();
        assert!(error.to_string().contains("byte limit"));
    }

    #[test]
    fn results_without_images_keep_the_exact_text() {
        let output = json::json!({ "rows": [1, 2], "note": "data:text/plain;base64,aGk=" });
        let query = outcome("id_1", "query", output.clone(), &[]);
        assert_eq!(query.text, json::to_string(&output).unwrap());
        let plain = outcome("id_2", "echo", json::json!("hello"), &[]);
        assert_eq!(plain.text, "hello");

        let messages = tool_turn_messages(&[query, plain]);
        assert_eq!(messages.len(), 1);
        assert_eq!(
            user_text(&messages[0]),
            vec![
                format!("<result id_1: {}>", json::to_string(&output).unwrap()),
                "<result id_2: hello>".to_string(),
            ]
        );
    }

    fn outcome(id: &str, name: &str, output: Value, images: &[(usize, &str)]) -> ToolCallOutcome {
        ToolCallOutcome::new(
            id.into(),
            None,
            name.into(),
            json::json!({}),
            &output,
            images
                .iter()
                .map(|(index, tag)| IndexedImage {
                    index: *index,
                    image: model_image(tag),
                })
                .collect(),
        )
    }

    #[test]
    fn tool_results_come_first_then_one_image_message() {
        let messages = tool_turn_messages(&[
            outcome(
                "call_a",
                "take_screenshot",
                json::json!("[image 1]"),
                &[(1, "A")],
            ),
            outcome("call_b", "lookup", json::json!({ "ok": true }), &[]),
            outcome(
                "call_c",
                "gallery",
                json::json!(["[image 2]", "[image 3]"]),
                &[(2, "B"), (3, "C")],
            ),
        ]);
        assert_eq!(messages.len(), 2);
        assert!(!is_tool_image_message(&messages[0]));
        assert!(is_tool_image_message(&messages[1]));
        assert_eq!(
            user_text(&messages[0]),
            vec![
                "<result call_a: [image 1]\n\n(Image attached in the next message: [image 1])>",
                "<result call_b: {\"ok\":true}>",
                "<result call_c: [\"[image 2]\",\"[image 3]\"]\n\n(Images attached in the next message: [image 2], [image 3])>",
            ]
        );
        assert_eq!(
            user_text(&messages[1]),
            vec![
                TOOL_IMAGE_MESSAGE_HEADER,
                "[image 1] from `take_screenshot` (call call_a):",
                "<image Base64(\"A\")>",
                "[image 2] from `gallery` (call call_c):",
                "<image Base64(\"B\")>",
                "[image 3] from `gallery` (call call_c):",
                "<image Base64(\"C\")>",
            ]
        );
    }

    #[test]
    fn image_messages_persist_as_history_images() {
        let message = image_message(1);
        let history = HistoryMessage::from(message);
        assert_eq!(history.role, Role::User);
        let MessageContent::Contents(parts) = history.content else {
            panic!("expected content parts");
        };
        assert!(parts.iter().any(|part| matches!(
            part,
            Content::Image { image_url, .. } if image_url.url.starts_with("data:image/png;base64,")
        )));
        let restored = rig::message::Message::try_from(HistoryMessage {
            role: Role::User,
            content: MessageContent::Contents(parts),
            name: None,
            tool_calls: None,
            tool_call_id: None,
            annotations: None,
        })
        .unwrap();
        assert!(is_tool_image_message(&restored));
        assert_eq!(count_images(&[restored]), 1);
    }

    #[test]
    fn pruning_waits_for_slack_then_keeps_the_newest_images() {
        let user_upload = Message::User {
            content: OneOrMany::one(UserContent::image_base64(
                "upload",
                Some(ImageMediaType::PNG),
                None,
            )),
        };
        let mut history = vec![user_upload];
        for _ in 0..TOOL_IMAGE_KEEP_RECENT + TOOL_IMAGE_PRUNE_SLACK {
            history.push(image_message(1));
            assert_eq!(
                prune_tool_images(
                    &mut history,
                    TOOL_IMAGE_KEEP_RECENT,
                    TOOL_IMAGE_PRUNE_SLACK
                ),
                0
            );
        }
        history.push(image_message(1));
        let pruned = prune_tool_images(&mut history, TOOL_IMAGE_KEEP_RECENT, TOOL_IMAGE_PRUNE_SLACK);
        assert_eq!(pruned, 3);
        assert_eq!(count_images(&history), 1 + TOOL_IMAGE_KEEP_RECENT);
        assert!(matches!(
            &history[0],
            Message::User { content } if matches!(content.first_ref(), UserContent::Image(_))
        ));
        assert!(user_text(&history[1]).contains(&PRUNED_TOOL_IMAGE_STUB.to_string()));
        assert!(user_text(&history[6]).iter().all(|part| part != PRUNED_TOOL_IMAGE_STUB));
    }

    #[test]
    fn pruning_never_drops_images_the_model_has_not_seen() {
        let mut history = vec![image_message(1), image_message(7)];
        let pruned = prune_tool_images(&mut history, TOOL_IMAGE_KEEP_RECENT, TOOL_IMAGE_PRUNE_SLACK);
        assert_eq!(pruned, 1);
        assert_eq!(count_images(&history), 7);
    }
}

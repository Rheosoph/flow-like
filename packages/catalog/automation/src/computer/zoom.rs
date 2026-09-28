#![cfg_attr(not(feature = "execute"), allow(dead_code))]

use super::ocr::{InputRect, PixelBox};
use crate::types::handles::AutomationSession;
use crate::types::screen_frame::ScreenFrame;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{async_trait, json::json};

pub(crate) fn frame_rect(frame: &ScreenFrame) -> InputRect {
    InputRect {
        x: frame.x,
        y: frame.y,
        width: frame.width,
        height: frame.height,
    }
}

pub(crate) fn window_rect(window: &super::window::WindowInfo) -> InputRect {
    InputRect {
        x: window.x,
        y: window.y,
        width: window.width,
        height: window.height,
    }
}

/// Pixel rectangle of `frame`'s image covering the part of `rect` that lies on it.
pub(crate) fn pixel_crop(frame: &ScreenFrame, rect: &InputRect) -> Option<PixelBox> {
    let visible = frame_rect(frame).intersection(rect)?;
    let (sx, sy) = (frame.scale_x(), frame.scale_y());
    let left = visible.x as i64 - frame.x as i64;
    let top = visible.y as i64 - frame.y as i64;
    let px0 = (left as f64 * sx).floor().max(0.0) as u32;
    let py0 = (top as f64 * sy).floor().max(0.0) as u32;
    let px1 = (((left + visible.width as i64) as f64 * sx).ceil() as u32).min(frame.pixel_width);
    let py1 = (((top + visible.height as i64) as f64 * sy).ceil() as u32).min(frame.pixel_height);
    (px1 > px0 && py1 > py0).then(|| PixelBox {
        x: px0,
        y: py0,
        width: px1 - px0,
        height: py1 - py0,
    })
}

/// Frame of a crop enlarged by an integer factor; pixels found in the enlarged image map back
/// through it with `pixel_to_input`.
pub(crate) fn upscaled_frame(
    frame: &ScreenFrame,
    factor: u32,
) -> flow_like_types::Result<ScreenFrame> {
    let width = frame.pixel_width.checked_mul(factor);
    let height = frame.pixel_height.checked_mul(factor);
    match (width, height) {
        (Some(width), Some(height)) if width as u64 * height as u64 <= 40_000_000 => {
            frame.resized(width, height)
        }
        _ => Err(flow_like_types::anyhow!(
            "Upscaling a {}x{} capture by {} exceeds the 40 megapixel limit",
            frame.pixel_width,
            frame.pixel_height,
            factor
        )),
    }
}

#[cfg(feature = "execute")]
pub(crate) fn capture_display(
    index: u32,
) -> flow_like_types::Result<(image::RgbaImage, ScreenFrame)> {
    let monitors = xcap::Monitor::all()
        .map_err(|e| flow_like_types::anyhow!("Cannot enumerate displays: {}", e))?;
    let monitor = monitors.get(index as usize).ok_or_else(|| {
        flow_like_types::anyhow!(
            "Display index {} not found; {} display(s) are connected",
            index,
            monitors.len()
        )
    })?;
    crate::types::screen_frame::capture_display_with_frame(monitor, Some(index))
}

/// Captures an input-space rectangle from the display it overlaps most, at the display's
/// native pixel density. Parts outside that display are clipped; the frame says what was kept.
#[cfg(feature = "execute")]
pub(crate) fn capture_input_rect(
    rect: InputRect,
) -> flow_like_types::Result<(image::RgbaImage, ScreenFrame)> {
    if rect.width == 0 || rect.height == 0 {
        return Err(flow_like_types::anyhow!(
            "Capture region {}x{} at ({}, {}) is empty",
            rect.width,
            rect.height,
            rect.x,
            rect.y
        ));
    }
    let monitors = xcap::Monitor::all()
        .map_err(|e| flow_like_types::anyhow!("Cannot enumerate displays: {}", e))?;
    let mut best: Option<(usize, u64)> = None;
    for (index, monitor) in monitors.iter().enumerate() {
        let (x, y, width, height) = crate::types::screen_match::monitor_input_bounds(monitor)?;
        let overlap = InputRect {
            x,
            y,
            width,
            height,
        }
        .intersection(&rect)
        .map_or(0, |r| r.area());
        if overlap > best.map_or(0, |b| b.1) {
            best = Some((index, overlap));
        }
    }
    let (index, _) = best.ok_or_else(|| {
        flow_like_types::anyhow!(
            "Region {}x{} at ({}, {}) does not overlap any display",
            rect.width,
            rect.height,
            rect.x,
            rect.y
        )
    })?;
    let (image, frame) = crate::types::screen_frame::capture_display_with_frame(
        &monitors[index],
        Some(index as u32),
    )?;
    let crop = pixel_crop(&frame, &rect).ok_or_else(|| {
        flow_like_types::anyhow!(
            "Region {}x{} at ({}, {}) is not visible on display {}",
            rect.width,
            rect.height,
            rect.x,
            rect.y,
            index
        )
    })?;
    let cropped =
        image::imageops::crop_imm(&image, crop.x, crop.y, crop.width, crop.height).to_image();
    Ok((
        cropped,
        frame.crop(crop.x, crop.y, crop.width, crop.height)?,
    ))
}

#[cfg(feature = "execute")]
pub(crate) async fn capture_input_rect_async(
    rect: InputRect,
) -> flow_like_types::Result<(image::RgbaImage, ScreenFrame)> {
    tokio::task::spawn_blocking(move || capture_input_rect(rect)).await?
}

#[cfg(feature = "execute")]
pub(crate) async fn capture_display_async(
    index: u32,
) -> flow_like_types::Result<(image::RgbaImage, ScreenFrame)> {
    tokio::task::spawn_blocking(move || capture_display(index)).await?
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerZoomNode;

impl ComputerZoomNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ComputerZoomNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_zoom",
            "Zoom Screen Region",
            "Captures a desktop rectangle at the display's full pixel density, optionally enlarged, to read small text or inspect details. Coordinates found in the image map back to the desktop through the frame",
            "Automation/Computer/Capture",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "zoom");
        node.add_icon("/flow/icons/computer.svg");
        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(5)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        for (name, friendly, description, default) in [
            ("x", "X", "Left edge in desktop input coordinates", 0),
            ("y", "Y", "Top edge in desktop input coordinates", 0),
            ("width", "Width", "Width in desktop input coordinates", 400),
            (
                "height",
                "Height",
                "Height in desktop input coordinates",
                300,
            ),
        ] {
            node.add_input_pin(name, friendly, description, VariableType::Integer)
                .set_default_value(Some(json!(default)));
        }
        node.add_input_pin(
            "upscale",
            "Upscale",
            "Enlarge the capture by this factor (1–4) for tiny text",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)))
        .set_options(
            PinOptions::new()
                .set_range((1.0, 4.0))
                .set_step(1.0)
                .build(),
        );

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();
        node.add_output_pin(
            "image",
            "Image",
            "The captured region",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();
        node.add_output_pin(
            "frame",
            "Frame",
            "Desktop rectangle and pixel size of the image; map image points back with it",
            VariableType::Struct,
        )
        .set_schema::<ScreenFrame>();
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let rect = InputRect {
            x: i32::try_from(context.evaluate_pin::<i64>("x").await?)?,
            y: i32::try_from(context.evaluate_pin::<i64>("y").await?)?,
            width: u32::try_from(context.evaluate_pin::<i64>("width").await?)
                .map_err(|_| flow_like_types::anyhow!("Zoom width must be positive"))?,
            height: u32::try_from(context.evaluate_pin::<i64>("height").await?)
                .map_err(|_| flow_like_types::anyhow!("Zoom height must be positive"))?,
        };
        let upscale: i64 = context.evaluate_pin("upscale").await?;
        if !(1..=4).contains(&upscale) {
            return Err(flow_like_types::anyhow!(
                "Upscale must be between 1 and 4, got {}",
                upscale
            ));
        }
        let factor = upscale as u32;

        let (picture, frame) = tokio::task::spawn_blocking(
            move || -> flow_like_types::Result<(image::RgbaImage, ScreenFrame)> {
                let (capture, frame) = capture_input_rect(rect)?;
                if factor == 1 {
                    return Ok((capture, frame));
                }
                let frame = upscaled_frame(&frame, factor)?;
                let enlarged = image::imageops::resize(
                    &capture,
                    frame.pixel_width,
                    frame.pixel_height,
                    image::imageops::FilterType::Lanczos3,
                );
                Ok((enlarged, frame))
            },
        )
        .await??;

        let node_image = NodeImage::new(context, image::DynamicImage::ImageRgba8(picture)).await;
        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("image", json!(node_image)).await?;
        context.set_pin_value("frame", json!(frame)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retina_secondary() -> ScreenFrame {
        ScreenFrame::new(Some(1), (-1440, -200, 1440, 900), (2880, 1800)).unwrap()
    }

    #[test]
    fn input_rect_crops_native_pixels_and_maps_back() {
        let frame = retina_secondary();
        let rect = InputRect {
            x: -1400,
            y: -150,
            width: 100,
            height: 50,
        };
        let crop = pixel_crop(&frame, &rect).unwrap();
        assert_eq!(
            crop,
            PixelBox {
                x: 80,
                y: 100,
                width: 200,
                height: 100
            }
        );
        let zoomed = frame.crop(crop.x, crop.y, crop.width, crop.height).unwrap();
        assert_eq!(
            (zoomed.x, zoomed.y, zoomed.width, zoomed.height),
            (-1400, -150, 100, 50)
        );
        let enlarged = upscaled_frame(&zoomed, 3).unwrap();
        assert_eq!((enlarged.pixel_width, enlarged.pixel_height), (600, 300));
        assert_eq!(
            enlarged.pixel_to_input(300.0, 150.0).unwrap(),
            (-1350, -125)
        );
    }

    #[test]
    fn regions_are_clipped_to_the_display() {
        let frame = retina_secondary();
        let spilling = InputRect {
            x: -20,
            y: 650,
            width: 100,
            height: 100,
        };
        assert_eq!(
            pixel_crop(&frame, &spilling),
            Some(PixelBox {
                x: 2840,
                y: 1700,
                width: 40,
                height: 100
            })
        );
        let outside = InputRect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        assert_eq!(pixel_crop(&frame, &outside), None);
        assert!(upscaled_frame(&frame, 4).is_err());
    }
}

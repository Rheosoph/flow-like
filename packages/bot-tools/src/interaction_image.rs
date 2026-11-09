//! A separate annotated image leaves the captured pixels available for comparison.

use std::io::Cursor;

use flow_like_types::image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use flow_like_types::interaction_observation::{
    InteractionObservation, InteractionPoint, InteractionRect, PointerSource,
};

use crate::outcome::{ToolImage, ToolOutput};

const OBSERVED: Rgba<u8> = Rgba([37, 99, 235, 255]);
const INTENDED: Rgba<u8> = Rgba([234, 88, 12, 255]);
const FOCUS: Rgba<u8> = Rgba([8, 145, 178, 255]);
pub(crate) const LEGEND: &str = "The first image is the clean capture. The second shows action feedback on the same pixels: blue cross = observed cursor; orange cross = last dispatched browser pointer; orange ring/box = intended target; cyan box = focus. Markers do not prove a click reached its target. Observation positions and bounds are fractions of the image; mapping is the desktop input rectangle.";

fn point(at: InteractionPoint, width: u32, height: u32) -> Option<(i32, i32)> {
    if !at.x.is_finite()
        || !at.y.is_finite()
        || !(0.0..=1.0).contains(&at.x)
        || !(0.0..=1.0).contains(&at.y)
    {
        return None;
    }
    Some((
        (at.x * width as f64)
            .floor()
            .min(width.saturating_sub(1) as f64) as i32,
        (at.y * height as f64)
            .floor()
            .min(height.saturating_sub(1) as f64) as i32,
    ))
}

fn pixel(image: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < image.width() && (y as u32) < image.height() {
        image.put_pixel(x as u32, y as u32, color);
    }
}

fn ring(image: &mut RgbaImage, x: i32, y: i32, radius: i32, color: Rgba<u8>) {
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let distance = dx * dx + dy * dy;
            if distance <= radius * radius && distance >= (radius - 2) * (radius - 2) {
                pixel(image, x + dx, y + dy, color);
            }
        }
    }
}

fn rectangle(image: &mut RgbaImage, rect: InteractionRect, color: Rgba<u8>) -> bool {
    if ![rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|n| n.is_finite())
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        return false;
    }
    let left = rect.x.max(0.0);
    let top = rect.y.max(0.0);
    let right = (rect.x + rect.width).min(1.0);
    let bottom = (rect.y + rect.height).min(1.0);
    if left >= right || top >= bottom {
        return false;
    }
    let Some((x0, y0)) = point(
        InteractionPoint { x: left, y: top },
        image.width(),
        image.height(),
    ) else {
        return false;
    };
    let Some((x1, y1)) = point(
        InteractionPoint {
            x: right,
            y: bottom,
        },
        image.width(),
        image.height(),
    ) else {
        return false;
    };
    for x in x0..=x1 {
        pixel(image, x, y0, color);
        pixel(image, x, y1, color);
    }
    for y in y0..=y1 {
        pixel(image, x0, y, color);
        pixel(image, x1, y, color);
    }
    true
}

fn annotated(png: &[u8], observation: &InteractionObservation) -> Option<Vec<u8>> {
    let mut image = flow_like_types::image::load_from_memory(png)
        .ok()?
        .to_rgba8();
    let mut drawn = false;
    if let Some(rect) = observation.focus.as_ref().and_then(|focus| focus.bounds) {
        drawn |= rectangle(&mut image, rect, FOCUS);
    }
    if let Some(action) = observation
        .action
        .as_ref()
        .filter(|action| observation.captured_at_ms.saturating_sub(action.at_ms) <= 8_000)
    {
        if let Some(rect) = action.target_bounds {
            drawn |= rectangle(&mut image, rect, INTENDED);
        }
        if let Some((x, y)) = action
            .target
            .and_then(|p| point(p, image.width(), image.height()))
        {
            ring(&mut image, x, y, 12, INTENDED);
            drawn = true;
        }
    }
    if let Some(pointer) = &observation.pointer
        && let Some((x, y)) = point(pointer.position, image.width(), image.height())
    {
        let color = match pointer.source {
            PointerSource::Observed => OBSERVED,
            PointerSource::Dispatched => INTENDED,
        };
        ring(&mut image, x, y, 5, color);
        for offset in -7..=7 {
            pixel(&mut image, x + offset, y, color);
            pixel(&mut image, x, y + offset, color);
        }
        drawn = true;
    }
    if !drawn {
        return None;
    }
    let mut result = Vec::new();
    DynamicImage::ImageRgba8(image)
        .write_to(&mut Cursor::new(&mut result), ImageFormat::Png)
        .ok()?;
    Some(result)
}

pub(crate) fn append(
    output: &mut ToolOutput,
    png: &[u8],
    observation: Option<&InteractionObservation>,
) -> bool {
    let Some(png) = observation.and_then(|observation| annotated(png, observation)) else {
        return false;
    };
    output.images.push(ToolImage {
        label: "Action feedback on the same capture".into(),
        mime: "image/png".into(),
        data: png.into(),
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::interaction_observation::InteractionPointer;

    #[test]
    fn observed_pointer_is_scaled_to_the_image_without_changing_the_original() {
        let mut original = Vec::new();
        DynamicImage::ImageRgba8(RgbaImage::new(200, 100))
            .write_to(&mut Cursor::new(&mut original), ImageFormat::Png)
            .unwrap();
        let observation = InteractionObservation {
            captured_at_ms: 10,
            width: 400,
            height: 200,
            pointer: Some(InteractionPointer {
                position: InteractionPoint { x: 0.75, y: 0.5 },
                source: PointerSource::Observed,
            }),
            focus: None,
            action: None,
            mapping: None,
        };
        let annotated =
            flow_like_types::image::load_from_memory(&annotated(&original, &observation).unwrap())
                .unwrap()
                .to_rgba8();
        assert_eq!(*annotated.get_pixel(150, 50), OBSERVED);
        assert_eq!(
            *flow_like_types::image::load_from_memory(&original)
                .unwrap()
                .to_rgba8()
                .get_pixel(150, 50),
            Rgba([0, 0, 0, 0])
        );
        assert_eq!(
            point(
                InteractionPoint {
                    x: f64::NAN,
                    y: 0.0
                },
                200,
                100
            ),
            None
        );
    }
}

use flow_like::flow_like_storage::files::store::FlowLikeStore;
use flow_like::flow_like_storage::object_store::ObjectStoreExt;
use flow_like::flow_like_storage::object_store::{PutPayload, path::Path};
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;

use crate::functions::TauriFunctionError;

/// Capture a display region with the pointer at its center.
pub fn capture_region_image(
    x: i32,
    y: i32,
    region_size: u32,
) -> Result<DynamicImage, TauriFunctionError> {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        let monitors =
            xcap::Monitor::all().map_err(|error| TauriFunctionError::new(&error.to_string()))?;
        let monitor = find_monitor_at(x, y, &monitors)
            .ok_or_else(|| TauriFunctionError::new("No monitor at click coordinates"))?;
        let image = flow_like_catalog::automation_screen::capture_monitor(monitor)
            .map_err(|error| TauriFunctionError::new(&error.to_string()))?;
        let (origin_x, origin_y, width, height) =
            flow_like_catalog::automation_screen::monitor_input_bounds(monitor)
                .map_err(|error| TauriFunctionError::new(&error.to_string()))?;
        if width == 0 || height == 0 {
            return Err(TauriFunctionError::new("Display has no input bounds"));
        }
        let px = ((x as i64 - origin_x as i64) as f64 * image.width() as f64 / width as f64).round()
            as i32;
        let py = ((y as i64 - origin_y as i64) as f64 * image.height() as f64 / height as f64)
            .round() as i32;
        let half_x =
            (region_size.clamp(2, 500) as f64 * image.width() as f64 / width as f64 / 2.0) as i32;
        let half_y =
            (region_size.clamp(2, 500) as f64 * image.height() as f64 / height as f64 / 2.0) as i32;
        let (rx, ry, crop_width, crop_height) =
            centered_crop(px, py, half_x, half_y, image.width(), image.height()).ok_or_else(
                || {
                    TauriFunctionError::new(
                        "Click is at display edge; use its recorded coordinates",
                    )
                },
            )?;
        Ok(DynamicImage::ImageRgba8(image).crop_imm(rx, ry, crop_width, crop_height))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (x, y, region_size);
        Err(TauriFunctionError::new("Screen recording is unsupported"))
    }
}

fn centered_crop(
    x: i32,
    y: i32,
    half_x: i32,
    half_y: i32,
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    // Matching clicks the image center. Shrink both sides equally at display edges.
    let hx = half_x.min(x).min(width as i32 - x);
    let hy = half_y.min(y).min(height as i32 - y);
    if hx < 1 || hy < 1 {
        return None;
    }
    Some((
        (x - hx) as u32,
        (y - hy) as u32,
        (hx * 2) as u32,
        (hy * 2) as u32,
    ))
}

pub async fn store_region(
    image: DynamicImage,
    store: &FlowLikeStore,
    app_id: Option<&str>,
    board_id: Option<&str>,
) -> Result<String, TauriFunctionError> {
    let (Some(app_id), Some(board_id)) = (app_id, board_id) else {
        return Err(TauriFunctionError::new(
            "Screenshot templates need an application and board",
        ));
    };
    let artifact_id = flow_like_types::create_id();
    let path = Path::from(format!(
        "apps/{app_id}/upload/rpa/{board_id}/screenshots/{artifact_id}.png"
    ));
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, ImageFormat::Png)
        .map_err(|error| TauriFunctionError::new(&error.to_string()))?;
    flow_like_types::tokio::time::timeout(
        std::time::Duration::from_secs(10),
        store
            .as_generic()
            .put(&path, PutPayload::from(bytes.into_inner())),
    )
    .await
    .map_err(|_| TauriFunctionError::new("Screenshot upload timed out"))?
    .map_err(|error| TauriFunctionError::new(&error.to_string()))?;
    Ok(artifact_id)
}

#[allow(dead_code)] // full-screen RPA capture, implemented but not wired to any command or recording setting
pub async fn capture_full_screen(store: &FlowLikeStore) -> Result<String, TauriFunctionError> {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        // Scope Monitor usage (non-Send) before any .await
        let screenshot = {
            use xcap::Monitor;

            let monitors = Monitor::all().map_err(|e| TauriFunctionError::new(&e.to_string()))?;

            let primary = monitors
                .into_iter()
                .find(|m| m.is_primary().unwrap_or(false))
                .ok_or_else(|| TauriFunctionError::new("No primary monitor found"))?;

            flow_like_catalog::automation_screen::capture_monitor(&primary)
                .map_err(|e| TauriFunctionError::new(&e.to_string()))?
        };

        let artifact_id = flow_like_types::create_id();
        let path = Path::from(format!("recordings/screenshots/{}.png", artifact_id));

        let mut bytes = Vec::new();
        let mut cursor = Cursor::new(&mut bytes);
        DynamicImage::ImageRgba8(screenshot)
            .write_to(&mut cursor, ImageFormat::Png)
            .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

        store
            .as_generic()
            .put(&path, PutPayload::from(bytes))
            .await
            .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

        Ok(artifact_id)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = store;
        Err(TauriFunctionError::new(
            "Screenshot capture not supported on this platform",
        ))
    }
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn find_monitor_at(x: i32, y: i32, monitors: &[xcap::Monitor]) -> Option<&xcap::Monitor> {
    monitors.iter().find(|monitor| {
        let Ok((mx, my, width, height)) =
            flow_like_catalog::automation_screen::monitor_input_bounds(monitor)
        else {
            return false;
        };
        let (dx, dy) = (x as i64 - mx as i64, y as i64 - my as i64);
        dx >= 0 && dy >= 0 && dx < width as i64 && dy < height as i64
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_templates_keep_the_recorded_click_at_their_center() {
        for (x, y) in [(5, 80), (190, 145), (80, 5), (100, 75)] {
            let (left, top, width, height) = centered_crop(x, y, 75, 75, 200, 150).unwrap();
            assert_eq!(left + width / 2, x as u32);
            assert_eq!(top + height / 2, y as u32);
            assert!(left + width <= 200 && top + height <= 150);
        }
        assert!(centered_crop(0, 0, 75, 75, 200, 150).is_none());
    }
}

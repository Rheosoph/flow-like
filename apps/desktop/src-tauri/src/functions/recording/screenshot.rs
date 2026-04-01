use flow_like::flow_like_storage::files::store::FlowLikeStore;
use flow_like::flow_like_storage::object_store::{ObjectStore, PutPayload, path::Path};
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;

use crate::functions::TauriFunctionError;

/// Capture a region around the given coordinates and store it.
///
/// The screenshot is stored under:
/// - `apps/{app_id}/upload/rpa/{board_id}/screenshots/{artifact_id}.png` if both IDs are provided
/// - `recordings/screenshots/{artifact_id}.png` otherwise (fallback for offline/local)
///
/// This matches the execution upload path structure: get_upload_dir() returns board_dir/upload
/// where board_dir = apps/{app_id}.
/// Returns the artifact_id which can be used to construct the full path later.
pub async fn capture_region(
    x: i32,
    y: i32,
    region_size: u32,
    store: &FlowLikeStore,
    app_id: Option<&str>,
    board_id: Option<&str>,
) -> Result<String, TauriFunctionError> {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        use xcap::Monitor;

        let monitors = Monitor::all().map_err(|e| TauriFunctionError::new(&e.to_string()))?;

        let monitor = find_monitor_at(x, y, &monitors)
            .ok_or_else(|| TauriFunctionError::new("No monitor found at coordinates"))?;

        let screenshot = monitor
            .capture_image()
            .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

        // On HiDPI displays (e.g. macOS Retina), capture_image() returns a
        // physical-resolution image but monitor x/y/width/height are logical.
        // Scale logical coordinates to physical pixels for correct cropping.
        let scale = monitor.scale_factor().unwrap_or(1.0);

        let monitor_x = monitor.x().unwrap_or(0);
        let monitor_y = monitor.y().unwrap_or(0);

        let rel_x = ((x - monitor_x) as f32 * scale) as i32;
        let rel_y = ((y - monitor_y) as f32 * scale) as i32;
        let scaled_half = (region_size as f32 * scale / 2.0) as i32;
        let scaled_region = (region_size as f32 * scale) as u32;

        let rx = (rel_x - scaled_half).max(0) as u32;
        let ry = (rel_y - scaled_half).max(0) as u32;

        let max_width = screenshot.width().saturating_sub(rx);
        let max_height = screenshot.height().saturating_sub(ry);
        let crop_width = scaled_region.min(max_width);
        let crop_height = scaled_region.min(max_height);

        let dynamic_img = DynamicImage::ImageRgba8(screenshot);
        let cropped = dynamic_img.crop_imm(rx, ry, crop_width, crop_height);

        tracing::debug!(
            "Screenshot capture: click=({}, {}), scale={}, cropped={}x{} (physical pixels)",
            x,
            y,
            scale,
            crop_width,
            crop_height
        );

        let artifact_id = flow_like_types::create_id();

        let path = match (app_id, board_id) {
            (Some(aid), Some(bid)) => Path::from(format!(
                "apps/{}/upload/rpa/{}/screenshots/{}.png",
                aid, bid, artifact_id
            )),
            _ => Path::from(format!("recordings/screenshots/{}.png", artifact_id)),
        };

        let mut bytes = Vec::new();
        let mut cursor = Cursor::new(&mut bytes);
        cropped
            .write_to(&mut cursor, ImageFormat::Png)
            .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

        store
            .as_generic()
            .put(&path, PutPayload::from(bytes))
            .await
            .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

        tracing::debug!(" Saved screenshot to: {}", path);

        Ok(artifact_id)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (x, y, region_size, store, board_id);
        Err(TauriFunctionError::new(
            "Screenshot capture not supported on this platform",
        ))
    }
}

pub async fn capture_full_screen(store: &FlowLikeStore) -> Result<String, TauriFunctionError> {
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    {
        use xcap::Monitor;

        let monitors = Monitor::all().map_err(|e| TauriFunctionError::new(&e.to_string()))?;

        let primary = monitors
            .into_iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .ok_or_else(|| TauriFunctionError::new("No primary monitor found"))?;

        let screenshot = primary
            .capture_image()
            .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

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
    monitors.iter().find(|m| {
        let mx = m.x().unwrap_or(0);
        let my = m.y().unwrap_or(0);
        let mw = m.width().unwrap_or(0) as i32;
        let mh = m.height().unwrap_or(0) as i32;

        x >= mx && x < mx + mw && y >= my && y < my + mh
    })
}

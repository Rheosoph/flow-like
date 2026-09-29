use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Where a screenshot sits on the desktop. `x`/`y`/`width`/`height` are in the
/// coordinate system the mouse and keyboard nodes use; `pixel_width`/`pixel_height`
/// are the dimensions of the image that was produced. Every coordinate read off an
/// image (by a model, template match, OCR, a pixel probe) is mapped back through this.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct ScreenFrame {
    #[serde(default)]
    pub display_index: Option<u32>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

impl ScreenFrame {
    pub fn new(
        display_index: Option<u32>,
        input_rect: (i32, i32, u32, u32),
        pixel_size: (u32, u32),
    ) -> flow_like_types::Result<Self> {
        let frame = Self {
            display_index,
            x: input_rect.0,
            y: input_rect.1,
            width: input_rect.2,
            height: input_rect.3,
            pixel_width: pixel_size.0,
            pixel_height: pixel_size.1,
        };
        frame.validate()?;
        Ok(frame)
    }

    pub fn validate(&self) -> flow_like_types::Result<()> {
        if self.width == 0 || self.height == 0 || self.pixel_width == 0 || self.pixel_height == 0 {
            return Err(flow_like_types::anyhow!(
                "Screen frame has an empty area (input {}x{}, pixels {}x{})",
                self.width,
                self.height,
                self.pixel_width,
                self.pixel_height
            ));
        }
        Ok(())
    }

    pub fn scale_x(&self) -> f64 {
        self.pixel_width as f64 / self.width as f64
    }

    pub fn scale_y(&self) -> f64 {
        self.pixel_height as f64 / self.height as f64
    }

    pub fn contains_pixel(&self, px: f64, py: f64) -> bool {
        px.is_finite()
            && py.is_finite()
            && px >= 0.0
            && py >= 0.0
            && px < self.pixel_width as f64
            && py < self.pixel_height as f64
    }

    pub fn contains_input(&self, x: i32, y: i32) -> bool {
        let (dx, dy) = (x as i64 - self.x as i64, y as i64 - self.y as i64);
        dx >= 0 && dy >= 0 && dx < self.width as i64 && dy < self.height as i64
    }

    /// Maps a point in image pixels to desktop input coordinates.
    pub fn pixel_to_input(&self, px: f64, py: f64) -> flow_like_types::Result<(i32, i32)> {
        self.validate()?;
        if !self.contains_pixel(px, py) {
            return Err(flow_like_types::anyhow!(
                "Point ({px}, {py}) lies outside the {}x{} screenshot",
                self.pixel_width,
                self.pixel_height
            ));
        }
        let x = self.x as f64 + px / self.scale_x();
        let y = self.y as f64 + py / self.scale_y();
        Ok((x.round() as i32, y.round() as i32))
    }

    /// Maps desktop input coordinates to image pixels, or `None` when the point is off-image.
    pub fn input_to_pixel(&self, x: i32, y: i32) -> Option<(u32, u32)> {
        if self.validate().is_err() || !self.contains_input(x, y) {
            return None;
        }
        let px = ((x as i64 - self.x as i64) as f64 * self.scale_x()).floor();
        let py = ((y as i64 - self.y as i64) as f64 * self.scale_y()).floor();
        Some((
            (px as u32).min(self.pixel_width - 1),
            (py as u32).min(self.pixel_height - 1),
        ))
    }

    /// The frame of a pixel sub-rectangle of this image.
    pub fn crop(&self, px: u32, py: u32, pw: u32, ph: u32) -> flow_like_types::Result<Self> {
        self.validate()?;
        if pw == 0
            || ph == 0
            || px.checked_add(pw).is_none_or(|end| end > self.pixel_width)
            || py.checked_add(ph).is_none_or(|end| end > self.pixel_height)
        {
            return Err(flow_like_types::anyhow!(
                "Crop {pw}x{ph} at ({px}, {py}) does not fit the {}x{} screenshot",
                self.pixel_width,
                self.pixel_height
            ));
        }
        let (sx, sy) = (self.scale_x(), self.scale_y());
        let x0 = self.x as f64 + px as f64 / sx;
        let y0 = self.y as f64 + py as f64 / sy;
        let x1 = self.x as f64 + (px + pw) as f64 / sx;
        let y1 = self.y as f64 + (py + ph) as f64 / sy;
        Self::new(
            self.display_index,
            (
                x0.floor() as i32,
                y0.floor() as i32,
                ((x1 - x0.floor()).ceil() as u32).max(1),
                ((y1 - y0.floor()).ceil() as u32).max(1),
            ),
            (pw, ph),
        )
    }

    /// Same desktop rectangle rendered at a different pixel size (e.g. downscaled for a model).
    pub fn resized(&self, pixel_width: u32, pixel_height: u32) -> flow_like_types::Result<Self> {
        Self::new(
            self.display_index,
            (self.x, self.y, self.width, self.height),
            (pixel_width, pixel_height),
        )
    }

    /// Pixel rectangle `(px, py, pw, ph)` of this image covering a desktop rectangle that must
    /// lie entirely inside the frame.
    pub fn input_rect_to_pixels(
        &self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    ) -> flow_like_types::Result<(u32, u32, u32, u32)> {
        self.validate()?;
        let (x0, y0) = (x as i64 - self.x as i64, y as i64 - self.y as i64);
        let (x1, y1) = (x0 + width as i64, y0 + height as i64);
        if width == 0
            || height == 0
            || x0 < 0
            || y0 < 0
            || x1 > self.width as i64
            || y1 > self.height as i64
        {
            return Err(flow_like_types::anyhow!(
                "Desktop region {}x{} at ({}, {}) does not fit display area {}x{} at ({}, {})",
                width,
                height,
                x,
                y,
                self.width,
                self.height,
                self.x,
                self.y
            ));
        }
        let (sx, sy) = (self.scale_x(), self.scale_y());
        let px0 = ((x0 as f64 * sx).floor() as u32).min(self.pixel_width - 1);
        let py0 = ((y0 as f64 * sy).floor() as u32).min(self.pixel_height - 1);
        let px1 = ((x1 as f64 * sx).ceil() as u32).clamp(px0 + 1, self.pixel_width);
        let py1 = ((y1 as f64 * sy).ceil() as u32).clamp(py0 + 1, self.pixel_height);
        Ok((px0, py0, px1 - px0, py1 - py0))
    }
}

/// Picks a display from `xcap::Monitor::all()`: `-1` is the primary display, otherwise the index.
#[cfg(feature = "execute")]
pub fn select_monitor(
    monitors: &[xcap::Monitor],
    index: i64,
) -> flow_like_types::Result<(usize, &xcap::Monitor)> {
    let position = if index == -1 {
        monitors
            .iter()
            .position(|m| m.is_primary().unwrap_or(false))
            .or((!monitors.is_empty()).then_some(0))
    } else {
        usize::try_from(index)
            .ok()
            .filter(|position| *position < monitors.len())
    };
    position
        .map(|position| (position, &monitors[position]))
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "Display index {} is unavailable ({} displays connected; -1 selects the primary)",
                index,
                monitors.len()
            )
        })
}

/// Pixel size a capture of this display produces, computed without capturing it.
#[cfg(feature = "execute")]
pub fn monitor_pixel_size(monitor: &xcap::Monitor) -> flow_like_types::Result<(u32, u32)> {
    let (_, _, width, height) = crate::types::screen_match::monitor_input_bounds(monitor)?;
    #[cfg(target_os = "macos")]
    let scale = crate::types::screen_match::monitor_scale_factor(monitor)? as f64;
    #[cfg(target_os = "linux")]
    let scale = if crate::computer::native::input::wayland() {
        crate::types::screen_match::monitor_scale_factor(monitor)? as f64
    } else {
        1.0
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let scale: f64 = 1.0;
    if !scale.is_finite() || scale <= 0.0 {
        return Err(flow_like_types::anyhow!(
            "Display reports an invalid scale factor {}",
            scale
        ));
    }
    Ok((
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    ))
}

/// Frame of a display from its geometry, for callers that need the mapping without a capture.
#[cfg(feature = "execute")]
pub fn monitor_frame(
    monitor: &xcap::Monitor,
    display_index: Option<u32>,
) -> flow_like_types::Result<ScreenFrame> {
    let bounds = crate::types::screen_match::monitor_input_bounds(monitor)?;
    ScreenFrame::new(display_index, bounds, monitor_pixel_size(monitor)?)
}

/// Captures a display by index (`-1` = primary) together with its frame. Blocking.
#[cfg(feature = "execute")]
pub fn capture_display(index: i64) -> flow_like_types::Result<(image::RgbaImage, ScreenFrame)> {
    let monitors = xcap::Monitor::all()
        .map_err(|e| flow_like_types::anyhow!("Failed to enumerate displays: {}", e))?;
    let (position, monitor) = select_monitor(&monitors, index)?;
    capture_display_with_frame(monitor, Some(position as u32))
        .map_err(|e| flow_like_types::anyhow!("Failed to capture display {}: {}", position, e))
}

/// Frames of every connected display, from geometry only. Blocking.
#[cfg(feature = "execute")]
pub fn display_frames() -> flow_like_types::Result<Vec<ScreenFrame>> {
    xcap::Monitor::all()
        .map_err(|e| flow_like_types::anyhow!("Failed to enumerate displays: {}", e))?
        .iter()
        .enumerate()
        .map(|(position, monitor)| monitor_frame(monitor, Some(position as u32)))
        .collect()
}

/// Captures a desktop rectangle in input coordinates from the display that contains it. When
/// `display_index` is `Some`, only that display (`-1` = primary) is used. Blocking.
#[cfg(feature = "execute")]
pub fn capture_input_region(
    display_index: Option<i64>,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> flow_like_types::Result<(image::RgbaImage, ScreenFrame)> {
    let monitors = xcap::Monitor::all()
        .map_err(|e| flow_like_types::anyhow!("Failed to enumerate displays: {}", e))?;
    let (position, monitor) = match display_index {
        Some(index) => select_monitor(&monitors, index)?,
        None => {
            let centre_x = x as i64 + width as i64 / 2;
            let centre_y = y as i64 + height as i64 / 2;
            let mut found = None;
            for (position, monitor) in monitors.iter().enumerate() {
                let frame = monitor_frame(monitor, Some(position as u32))?;
                if i32::try_from(centre_x)
                    .ok()
                    .zip(i32::try_from(centre_y).ok())
                    .is_some_and(|(cx, cy)| frame.contains_input(cx, cy))
                {
                    found = Some((position, monitor));
                    break;
                }
            }
            found.ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Desktop region {}x{} at ({}, {}) is not on a connected display",
                    width,
                    height,
                    x,
                    y
                )
            })?
        }
    };
    let (image, frame) = capture_display_with_frame(monitor, Some(position as u32))?;
    let (px, py, pw, ph) = frame.input_rect_to_pixels(x, y, width, height)?;
    let cropped = image::imageops::crop_imm(&image, px, py, pw, ph).to_image();
    Ok((cropped, frame.crop(px, py, pw, ph)?))
}

/// Largest image size a vision model should receive: long edge and total pixel budget.
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

/// Downscales an image to the model budget and returns the frame of the resized image.
#[cfg(feature = "execute")]
pub fn fit_image_for_model(
    image: &image::DynamicImage,
    frame: &ScreenFrame,
    budget: ModelImageBudget,
) -> flow_like_types::Result<(image::DynamicImage, ScreenFrame)> {
    let (w, h) = budget.fit(image.width(), image.height());
    if (w, h) == (image.width(), image.height()) {
        return Ok((image.clone(), frame.clone()));
    }
    let resized = image.resize_exact(w, h, image::imageops::FilterType::Triangle);
    Ok((resized, frame.resized(w, h)?))
}

/// Captures one display and returns the image together with its frame.
#[cfg(feature = "execute")]
pub fn capture_display_with_frame(
    monitor: &xcap::Monitor,
    display_index: Option<u32>,
) -> flow_like_types::Result<(image::RgbaImage, ScreenFrame)> {
    let image = crate::types::screen_match::capture_monitor(monitor)?;
    let bounds = crate::types::screen_match::monitor_input_bounds(monitor)?;
    let frame = ScreenFrame::new(display_index, bounds, (image.width(), image.height()))?;
    Ok((image, frame))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn retina_secondary() -> ScreenFrame {
        ScreenFrame::new(Some(1), (-1440, -200, 1440, 900), (2880, 1800)).unwrap()
    }

    #[test]
    fn retina_pixels_map_to_input_space_on_negative_origin_display() {
        let frame = retina_secondary();
        assert_eq!(frame.pixel_to_input(1000.0, 600.0).unwrap(), (-940, 100));
        assert_eq!(frame.input_to_pixel(-940, 100), Some((1000, 600)));
        assert!(frame.pixel_to_input(2880.0, 0.0).is_err());
        assert_eq!(frame.input_to_pixel(0, 0), None);
    }

    #[test]
    fn crop_and_resize_keep_desktop_geometry() {
        let frame = retina_secondary();
        let crop = frame.crop(200, 100, 400, 200).unwrap();
        assert_eq!(
            (crop.x, crop.y, crop.width, crop.height),
            (-1340, -150, 200, 100)
        );
        assert_eq!(crop.pixel_to_input(0.0, 0.0).unwrap(), (-1340, -150));
        let small = frame.resized(1440, 900).unwrap();
        assert_eq!(small.pixel_to_input(500.0, 300.0).unwrap(), (-940, 100));
        assert!(frame.crop(2800, 0, 100, 10).is_err());
    }

    #[test]
    fn desktop_rectangles_map_to_covering_pixel_rectangles() {
        let frame = retina_secondary();
        assert_eq!(
            frame.input_rect_to_pixels(-1440, -200, 1440, 900).unwrap(),
            (0, 0, 2880, 1800)
        );
        assert_eq!(
            frame.input_rect_to_pixels(-940, 100, 10, 5).unwrap(),
            (1000, 600, 20, 10)
        );
        let crop = frame.crop(1000, 600, 20, 10).unwrap();
        assert_eq!(
            (crop.x, crop.y, crop.width, crop.height),
            (-940, 100, 10, 5)
        );
        assert!(frame.input_rect_to_pixels(-10, 0, 20, 20).is_err());
        assert!(frame.input_rect_to_pixels(-940, 100, 0, 5).is_err());
        let fractional = ScreenFrame::new(None, (0, 0, 1000, 1000), (1500, 1500)).unwrap();
        assert_eq!(
            fractional.input_rect_to_pixels(1, 1, 1, 1).unwrap(),
            (1, 1, 2, 2)
        );
    }

    #[test]
    fn model_budget_limits_edge_and_area() {
        let budget = ModelImageBudget::default();
        let (w, h) = budget.fit(2880, 1800);
        assert!(w <= 1568 && (w as u64 * h as u64) <= 1_150_000);
        assert_eq!(budget.fit(800, 600), (800, 600));
    }
}

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub(crate) fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }

    pub(crate) fn distance_to(&self, x: f64, y: f64) -> f64 {
        let dx = (self.x - x).max(0.0).max(x - (self.x + self.width));
        let dy = (self.y - y).max(0.0).max(y - (self.y + self.height));
        dx.hypot(dy)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HardwareNotch {
    pub left: f64,
    pub right: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotchLayout {
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    pub menu_bar_height: f64,
    pub notch: Option<HardwareNotch>,
    /// The display shows a fullscreen Space: the den stays out of sight until the cursor reaches
    /// the top edge.
    pub fullscreen: bool,
    pub platform: &'static str,
}

/// Physical rows left free at the bottom. Windows takes a window that covers a whole monitor for
/// a fullscreen app and stops keeping the taskbar on top.
const UNCOVERED_ROWS: u32 = if cfg!(windows) { 1 } else { 0 };

pub(crate) const PLATFORM: &str = if cfg!(target_os = "macos") {
    "macos"
} else if cfg!(windows) {
    "windows"
} else {
    "linux"
};

/// The display the notch window covers. `origin_*` and `scale` are the monitor's physical origin
/// and device scale (the space of `cursor_position` and `outer_position`); `width`/`height` are
/// logical CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    pub origin_x: f64,
    pub origin_y: f64,
    pub scale: f64,
    pub width: f64,
    pub height: f64,
}

/// Origin and divisor that map the window list's bounds to window-local logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WindowSpace {
    pub origin_x: f64,
    pub origin_y: f64,
    pub divisor: f64,
}

impl Geometry {
    pub(crate) fn from_monitor(position: (i32, i32), size: (u32, u32), scale: f64) -> Self {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        Self {
            origin_x: f64::from(position.0),
            origin_y: f64::from(position.1),
            scale,
            width: f64::from(size.0) / scale,
            height: f64::from(size.1.saturating_sub(UNCOVERED_ROWS)) / scale,
        }
    }

    pub(crate) fn cursor_to_local(&self, x: f64, y: f64) -> (f64, f64) {
        (
            (x - self.origin_x) / self.scale,
            (y - self.origin_y) / self.scale,
        )
    }

    /// macOS reports window bounds in global points whose origin is the primary display; the
    /// other systems report physical pixels.
    pub(crate) fn window_space(&self) -> WindowSpace {
        if cfg!(target_os = "macos") {
            WindowSpace {
                origin_x: 0.0,
                origin_y: 0.0,
                divisor: 1.0,
            }
        } else {
            WindowSpace {
                origin_x: self.origin_x,
                origin_y: self.origin_y,
                divisor: self.scale,
            }
        }
    }

    /// The overlay's own bounds in the window list's space, where that list includes it.
    pub(crate) fn overlay_rect(&self) -> Option<Rect> {
        (!cfg!(target_os = "macos")).then_some(Rect {
            x: self.origin_x,
            y: self.origin_y,
            width: self.width * self.scale,
            height: self.height * self.scale,
        })
    }

    pub(crate) fn layout(&self, metrics: PlatformMetrics) -> NotchLayout {
        NotchLayout {
            width: self.width,
            height: self.height,
            scale: self.scale,
            menu_bar_height: metrics.menu_bar_height,
            notch: metrics.notch,
            fullscreen: metrics.fullscreen,
            platform: PLATFORM,
        }
    }
}

impl WindowSpace {
    pub(crate) fn to_local(&self, rect: Rect) -> Rect {
        Rect {
            x: (rect.x - self.origin_x) / self.divisor,
            y: (rect.y - self.origin_y) / self.divisor,
            width: rect.width / self.divisor,
            height: rect.height / self.divisor,
        }
    }

    pub(crate) fn point_from_local(&self, x: f64, y: f64) -> (f64, f64) {
        (
            x * self.divisor + self.origin_x,
            y * self.divisor + self.origin_y,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PlatformMetrics {
    pub menu_bar_height: f64,
    pub notch: Option<HardwareNotch>,
    pub fullscreen: bool,
}

/// Window-local notch from the screen's top safe-area inset and the two menu-bar areas beside it.
pub(crate) fn hardware_notch(
    screen_min_x: f64,
    top_inset: f64,
    left_area_max_x: f64,
    left_area_width: f64,
    right_area_min_x: f64,
    right_area_width: f64,
) -> Option<HardwareNotch> {
    if top_inset <= 0.0 || left_area_width <= 0.0 || right_area_width <= 0.0 {
        return None;
    }
    let left = left_area_max_x - screen_min_x;
    let right = right_area_min_x - screen_min_x;
    (right > left).then_some(HardwareNotch {
        left,
        right,
        height: top_inset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_is_window_local_logical() {
        let geometry = Geometry::from_monitor((0, 0), (3024, 1964), 2.0);
        assert_eq!(geometry.width, 1512.0);
        assert_eq!(geometry.cursor_to_local(1512.0, 40.0), (756.0, 20.0));
        let shifted = Geometry::from_monitor((1920, 0), (3840, 2160), 1.5);
        assert_eq!(shifted.cursor_to_local(2220.0, 150.0), (200.0, 100.0));
    }

    #[test]
    fn invalid_scale_falls_back_to_one() {
        assert_eq!(Geometry::from_monitor((0, 0), (100, 50), 0.0).width, 100.0);
    }

    #[test]
    fn only_windows_leaves_a_row_of_the_monitor_uncovered() {
        let geometry = Geometry::from_monitor((0, 0), (1920, 1080), 1.0);
        assert_eq!(geometry.width, 1920.0);
        assert_eq!(geometry.height, if cfg!(windows) { 1079.0 } else { 1080.0 });
        assert_eq!(Geometry::from_monitor((0, 0), (10, 0), 1.0).height, 0.0);
    }

    #[test]
    fn the_layout_carries_the_fullscreen_space() {
        let geometry = Geometry::from_monitor((0, 0), (3024, 1964), 2.0);
        assert!(!geometry.layout(PlatformMetrics::default()).fullscreen);
        let metrics = PlatformMetrics {
            fullscreen: true,
            ..PlatformMetrics::default()
        };
        assert!(geometry.layout(metrics).fullscreen);
    }

    #[test]
    fn window_bounds_convert_with_the_platform_divisor() {
        let geometry = Geometry::from_monitor((100, 0), (3000, 2000), 2.0);
        let space = geometry.window_space();
        let local = space.to_local(Rect {
            x: 300.0,
            y: 80.0,
            width: 800.0,
            height: 400.0,
        });
        if cfg!(target_os = "macos") {
            assert_eq!(local.x, 300.0);
            assert_eq!(local.width, 800.0);
        } else {
            assert_eq!(local.x, 100.0);
            assert_eq!(local.width, 400.0);
        }
        let (px, py) = space.point_from_local(local.x, local.y);
        assert_eq!((px, py), (300.0, 80.0));
    }

    #[test]
    fn rect_hit_and_distance() {
        let rect = Rect {
            x: 10.0,
            y: 10.0,
            width: 20.0,
            height: 10.0,
        };
        assert!(rect.contains(10.0, 10.0));
        assert!(!rect.contains(30.0, 15.0));
        assert_eq!(rect.distance_to(15.0, 15.0), 0.0);
        assert_eq!(rect.distance_to(40.0, 15.0), 10.0);
        assert_eq!(rect.distance_to(33.0, 24.0), 5.0);
    }

    #[test]
    fn hardware_notch_needs_an_inset_and_both_areas() {
        let notch = hardware_notch(0.0, 38.0, 660.0, 660.0, 852.0, 660.0).unwrap();
        assert_eq!(
            (notch.left, notch.right, notch.height),
            (660.0, 852.0, 38.0)
        );
        assert!(hardware_notch(0.0, 0.0, 660.0, 660.0, 852.0, 660.0).is_none());
        assert!(hardware_notch(0.0, 38.0, 0.0, 0.0, 852.0, 660.0).is_none());
        assert!(hardware_notch(0.0, 38.0, 900.0, 900.0, 852.0, 660.0).is_none());
    }

    #[test]
    fn notch_is_relative_to_the_screen_origin() {
        let notch = hardware_notch(-100.0, 32.0, 560.0, 660.0, 752.0, 660.0).unwrap();
        assert_eq!((notch.left, notch.right), (660.0, 852.0));
    }
}

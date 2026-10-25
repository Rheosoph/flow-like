/// Rasterize at the next half-octave, then shrink to the exact viewport scale.
/// The raster is never enlarged and is at most sqrt(2) times the displayed size.
pub fn raster_zoom(zoom: f32) -> f32 {
    ((zoom.log2() * 2.0).ceil() * 0.5)
        .exp2()
        .min(f32::MAX)
        .max(zoom)
}

const ZOOM_SETTLE_MS: f64 = 150.0;

pub struct ZoomQuality {
    zoom: f32,
    changed_at: Option<f64>,
}

impl ZoomQuality {
    pub fn new(zoom: f32) -> Self {
        Self {
            zoom,
            changed_at: None,
        }
    }

    pub fn update(&mut self, zoom: f32, now_ms: f64) {
        if self.zoom != zoom {
            self.zoom = zoom;
            self.changed_at = (raster_zoom(zoom) != zoom).then_some(now_ms);
        }
    }

    pub fn settle_delay_ms(&mut self, now_ms: f64) -> Option<f64> {
        let changed_at = self.changed_at?;
        let remaining = ZOOM_SETTLE_MS - (now_ms - changed_at).max(0.0);
        if remaining > 0.0 {
            Some(remaining)
        } else {
            self.changed_at = None;
            None
        }
    }
}

pub struct TextCache<T> {
    raster_key: Option<(u32, u32)>,
    displayed_zoom: u32,
    title_room: Option<u32>,
    pub title: Option<T>,
    pub pins: Vec<T>,
    /// The hidden-pin count on a collapsed latch.
    pub latch: Option<T>,
    pub scaled_title: Option<T>,
    pub scaled_pins: Vec<T>,
    pub scaled_latch: Option<T>,
}

impl<T> Default for TextCache<T> {
    fn default() -> Self {
        Self {
            raster_key: None,
            displayed_zoom: 0,
            title_room: None,
            title: None,
            pins: Vec::new(),
            latch: None,
            scaled_title: None,
            scaled_pins: Vec::new(),
            scaled_latch: None,
        }
    }
}

impl<T> TextCache<T> {
    pub fn prepare(&mut self, zoom: f32, pixels_per_point: f32, settled: bool) -> f32 {
        let raster_zoom = if settled { zoom } else { raster_zoom(zoom) };
        let key = (raster_zoom.to_bits(), pixels_per_point.to_bits());
        let raster_changed = self.raster_key != Some(key);
        if raster_changed {
            self.raster_key = Some(key);
            self.title = None;
            self.pins.clear();
            self.latch = None;
        }
        if raster_changed || self.displayed_zoom != zoom.to_bits() {
            self.displayed_zoom = zoom.to_bits();
            self.scaled_title = None;
            self.scaled_pins.clear();
            self.scaled_latch = None;
        }
        raster_zoom
    }

    /// Status items in the header narrow the title; a new width lays it out again.
    pub fn fit_title(&mut self, room: f32) {
        if self.title_room != Some(room.to_bits()) {
            self.title_room = Some(room.to_bits());
            self.title = None;
            self.scaled_title = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_buckets_never_enlarge_text_and_bound_downsampling() {
        for i in 1..=10_000 {
            let zoom = 0.01 + i as f32 / 1000.0;
            let raster = raster_zoom(zoom);
            assert!(raster >= zoom);
            assert!(raster / zoom <= std::f32::consts::SQRT_2 + 0.0001);
            for font_size in [9.6, 10.0, 12.0] {
                let displayed = font_size * raster * (zoom / raster);
                assert!((displayed - font_size * zoom).abs() < 0.0001);
            }
        }
        for zoom in [0.125, 0.25, 0.5, 1.0, 2.0, 4.0] {
            assert_eq!(raster_zoom(zoom), zoom);
        }
    }

    #[test]
    fn full_detail_zoom_reuses_title_and_pin_rasters_but_rescales_meshes() {
        let mut cache = TextCache::default();
        assert_eq!(
            cache.prepare(0.55, 2.0, false),
            std::f32::consts::FRAC_1_SQRT_2
        );
        cache.title = Some("raster title");
        cache.pins = vec!["raster input", "raster output"];
        cache.scaled_title = Some("display title");
        cache.scaled_pins = vec!["display input", "display output"];

        cache.prepare(0.65, 2.0, false);
        assert_eq!(cache.title, Some("raster title"));
        assert_eq!(cache.pins, ["raster input", "raster output"]);
        assert!(cache.scaled_title.is_none());
        assert!(cache.scaled_pins.is_empty());

        cache.scaled_title = Some("new display title");
        cache.scaled_pins = vec!["new display input", "new display output"];
        cache.prepare(0.65, 2.0, false);
        assert_eq!(cache.scaled_title, Some("new display title"));
        assert_eq!(cache.scaled_pins.len(), 2);
    }

    #[test]
    fn compact_zoom_reuses_titles_and_retains_pin_rasters_for_full_detail() {
        let mut cache = TextCache::default();
        cache.prepare(0.5, 2.0, false);
        cache.title = Some("title");
        cache.pins = vec!["input", "output"];
        cache.prepare(0.45, 2.0, false);
        cache.prepare(0.4, 2.0, false);
        assert_eq!(cache.title, Some("title"));
        assert_eq!(cache.pins, ["input", "output"]);
        cache.prepare(0.5, 2.0, false);
        assert_eq!(cache.title, Some("title"));
        assert_eq!(cache.pins, ["input", "output"]);
    }

    #[test]
    fn bucket_and_dpr_changes_invalidate_rasters_and_display_meshes() {
        let mut cache = TextCache::default();
        for (zoom, dpr) in [(0.6, 1.0), (0.8, 1.0), (0.8, 2.0)] {
            cache.prepare(zoom, dpr, false);
            assert!(cache.title.is_none());
            assert!(cache.pins.is_empty());
            assert!(cache.scaled_title.is_none());
            assert!(cache.scaled_pins.is_empty());
            cache.title = Some("title");
            cache.pins = vec!["pin"];
            cache.scaled_title = Some("display title");
            cache.scaled_pins = vec!["display pin"];
        }
    }

    #[test]
    fn settled_zoom_restores_exact_raster_once_and_panning_retains_it() {
        let mut quality = ZoomQuality::new(1.0);
        let mut cache = TextCache::default();
        quality.update(1.2, 0.0);
        assert_eq!(quality.settle_delay_ms(0.0), Some(150.0));
        cache.prepare(1.2, 2.0, false);
        cache.title = Some("bucket title");
        cache.pins = vec!["bucket pin"];

        // The host resubmits the same zoom during a pan; this must not extend the deadline.
        quality.update(1.2, 100.0);
        assert_eq!(quality.settle_delay_ms(100.0), Some(50.0));
        assert_eq!(quality.settle_delay_ms(150.0), None);
        assert_eq!(cache.prepare(1.2, 2.0, true), 1.2);
        assert!(cache.title.is_none());
        assert!(cache.pins.is_empty());
        cache.title = Some("exact title");
        cache.pins = vec!["exact pin"];
        cache.scaled_title = Some("exact mesh");

        quality.update(1.2, 200.0);
        assert_eq!(quality.settle_delay_ms(200.0), None);
        cache.prepare(1.2, 2.0, true);
        assert_eq!(cache.title, Some("exact title"));
        assert_eq!(cache.pins, ["exact pin"]);
        assert_eq!(cache.scaled_title, Some("exact mesh"));
    }

    #[test]
    fn a_new_title_room_relays_only_the_title() {
        let mut cache = TextCache::default();
        cache.fit_title(124.0);
        cache.title = Some("title");
        cache.scaled_title = Some("display title");
        cache.pins = vec!["pin"];
        cache.fit_title(124.0);
        assert_eq!(cache.title, Some("title"));
        cache.fit_title(96.0);
        assert!(cache.title.is_none() && cache.scaled_title.is_none());
        assert_eq!(cache.pins, ["pin"]);
    }

    #[test]
    fn zoom_changes_restart_settling_but_exact_buckets_need_no_delayed_paint() {
        let mut quality = ZoomQuality::new(1.0);
        assert_eq!(quality.settle_delay_ms(0.0), None);
        quality.update(1.2, 0.0);
        quality.update(1.3, 100.0);
        assert_eq!(quality.settle_delay_ms(150.0), Some(100.0));
        quality.update(2.0, 200.0);
        assert_eq!(quality.settle_delay_ms(200.0), None);
    }
}

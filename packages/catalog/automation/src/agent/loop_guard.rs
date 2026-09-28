use crate::computer::stability::{changed_fraction, diff_thumbnail};
use std::time::Duration;

const FINGERPRINT_EDGE: u32 = 480;
const PIXEL_TOLERANCE: u8 = 10;
/// Share of fingerprint pixels that must differ before a screen counts as changed. Blinking
/// carets and ticking clocks stay below it; a typed word or an opened menu exceeds it.
const CHANGE_THRESHOLD: f64 = 0.0003;
pub(crate) const NOTE_AFTER: u32 = 3;
pub(crate) const STOP_AFTER: u32 = 5;

/// Downsampled grayscale of a screenshot, compared between observations.
pub(crate) fn fingerprint(image: &image::RgbaImage) -> image::GrayImage {
    diff_thumbnail(image, FINGERPRINT_EDGE)
}

pub(crate) fn screen_changed(before: &image::GrayImage, after: &image::GrayImage) -> bool {
    changed_fraction(before, after, PIXEL_TOLERANCE).map_or(true, |share| share >= CHANGE_THRESHOLD)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Progress {
    Normal,
    NoEffect(u32),
    Stuck(u32),
}

/// Counts consecutive turns whose input actions left the screen unchanged. Turns without
/// input (wait, zoom, failed calls) neither count nor reset the streak.
#[derive(Default)]
pub(crate) struct StuckDetector {
    streak: u32,
}

impl StuckDetector {
    pub(crate) fn record(&mut self, acted: bool, changed: bool) -> Progress {
        if !acted {
            return Progress::Normal;
        }
        if changed {
            self.streak = 0;
            return Progress::Normal;
        }
        self.streak += 1;
        match self.streak {
            streak if streak >= STOP_AFTER => Progress::Stuck(streak),
            streak if streak >= NOTE_AFTER => Progress::NoEffect(streak),
            _ => Progress::Normal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    Done,
    Failed,
    Stuck,
    MaxSteps,
    Timeout,
    NeedsInput,
}

impl Status {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Stuck => "stuck",
            Self::MaxSteps => "max_steps",
            Self::Timeout => "timeout",
            Self::NeedsInput => "needs_input",
        }
    }

    pub(crate) fn exec_pin(self) -> &'static str {
        match self {
            Self::Done => "exec_out",
            Self::NeedsInput => "exec_needs_input",
            Self::Failed | Self::Stuck | Self::MaxSteps | Self::Timeout => "exec_failed",
        }
    }
}

/// The budget that ran out before the next model call, if any.
pub(crate) fn exhausted(
    steps_taken: u32,
    max_steps: u32,
    elapsed: Duration,
    max_duration: Duration,
) -> Option<Status> {
    if elapsed >= max_duration {
        Some(Status::Timeout)
    } else if steps_taken >= max_steps {
        Some(Status::MaxSteps)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stuck_detector_notes_then_stops_and_resets_on_change() {
        let mut detector = StuckDetector::default();
        assert_eq!(detector.record(true, false), Progress::Normal);
        assert_eq!(detector.record(true, false), Progress::Normal);
        assert_eq!(detector.record(false, false), Progress::Normal);
        assert_eq!(detector.record(true, false), Progress::NoEffect(3));
        assert_eq!(detector.record(true, false), Progress::NoEffect(4));
        assert_eq!(detector.record(true, false), Progress::Stuck(5));

        let mut detector = StuckDetector::default();
        for _ in 0..4 {
            detector.record(true, false);
        }
        assert_eq!(detector.record(true, true), Progress::Normal);
        assert_eq!(detector.record(true, false), Progress::Normal);
    }

    #[test]
    fn budgets_stop_on_time_before_steps() {
        let minute = Duration::from_secs(60);
        assert_eq!(exhausted(3, 30, Duration::from_secs(10), minute), None);
        assert_eq!(
            exhausted(30, 30, Duration::from_secs(10), minute),
            Some(Status::MaxSteps)
        );
        assert_eq!(exhausted(30, 30, minute, minute), Some(Status::Timeout));
    }

    #[test]
    fn statuses_route_to_their_exec_pins() {
        assert_eq!(Status::Done.exec_pin(), "exec_out");
        assert_eq!(Status::NeedsInput.exec_pin(), "exec_needs_input");
        for status in [
            Status::Failed,
            Status::Stuck,
            Status::MaxSteps,
            Status::Timeout,
        ] {
            assert_eq!(status.exec_pin(), "exec_failed");
        }
        assert_eq!(Status::MaxSteps.as_str(), "max_steps");
    }

    fn screen(width: u32, height: u32) -> image::RgbaImage {
        image::RgbaImage::from_pixel(width, height, image::Rgba([240, 240, 240, 255]))
    }

    fn paint(image: &mut image::RgbaImage, x: u32, y: u32, w: u32, h: u32) {
        for py in y..y + h {
            for px in x..x + w {
                image.put_pixel(px, py, image::Rgba([20, 20, 20, 255]));
            }
        }
    }

    #[test]
    fn caret_blinks_are_not_changes_but_typed_words_are() {
        let before = screen(1440, 900);
        let mut caret = before.clone();
        paint(&mut caret, 700, 400, 2, 16);
        assert!(!screen_changed(&fingerprint(&before), &fingerprint(&caret)));

        let mut typed = before.clone();
        for glyph in 0..5 {
            paint(&mut typed, 700 + glyph * 9, 400, 6, 12);
        }
        assert!(screen_changed(&fingerprint(&before), &fingerprint(&typed)));
        assert!(screen_changed(
            &fingerprint(&before),
            &fingerprint(&screen(800, 600))
        ));
    }
}

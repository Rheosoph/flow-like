use flow_like_bot_tools::computer::PointerButton;
use serde::{Deserialize, Serialize};

/// What the owner does at the computer while taking over. Positions are pixels of the
/// frame the owner is looking at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputEvent {
    PointerMove {
        x: f64,
        y: f64,
    },
    PointerDown {
        x: f64,
        y: f64,
        button: PointerButton,
    },
    PointerUp {
        x: f64,
        y: f64,
        button: PointerButton,
    },
    Scroll {
        x: f64,
        y: f64,
        dx: i32,
        dy: i32,
    },
    /// A key or chord pressed once, such as `Enter`, `Backspace` or `Control+c`.
    Key {
        chord: String,
    },
    Text {
        text: String,
    },
}

impl InputEvent {
    pub fn position(&self) -> Option<(f64, f64)> {
        match self {
            Self::PointerMove { x, y }
            | Self::PointerDown { x, y, .. }
            | Self::PointerUp { x, y, .. }
            | Self::Scroll { x, y, .. } => Some((*x, *y)),
            Self::Key { .. } | Self::Text { .. } => None,
        }
    }

    /// The same event with its position as fractions of a frame `width` x `height`.
    pub fn normalized(&self, width: u32, height: u32) -> Option<(f64, f64)> {
        let (x, y) = self.position()?;
        let (w, h) = (width.max(1) as f64, height.max(1) as f64);
        Some(((x / w).clamp(0.0, 1.0), (y / h).clamp(0.0, 1.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_become_fractions_of_the_frame_and_stay_on_the_screen() {
        let down = InputEvent::PointerDown {
            x: 640.0,
            y: 200.0,
            button: PointerButton::Left,
        };
        assert_eq!(down.normalized(1280, 800), Some((0.5, 0.25)));
        let off = InputEvent::PointerMove { x: -5.0, y: 9000.0 };
        assert_eq!(off.normalized(1280, 800), Some((0.0, 1.0)));
        assert_eq!(InputEvent::Text { text: "a".into() }.normalized(1, 1), None);
    }

    #[test]
    fn events_round_trip_with_a_type_tag() {
        let event: InputEvent =
            serde_json::from_str(r#"{"type":"scroll","x":1,"y":2,"dx":0,"dy":-3}"#).unwrap();
        assert_eq!(
            event,
            InputEvent::Scroll {
                x: 1.0,
                y: 2.0,
                dx: 0,
                dy: -3
            }
        );
    }
}

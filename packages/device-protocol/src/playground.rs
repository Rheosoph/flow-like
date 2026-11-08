//! The live view of a bot's screen over a `TunnelTarget::BotPlayground` stream. Messages are
//! length-prefixed (`kind: u8`, `length: u32` big endian, payload) so they survive any
//! chunking of the tunnel stream. Frames never pass through the hub.

use crate::{ProtocolError, Result};
use flow_like_types_contracts::interaction::InteractionObservation;
use serde::{Deserialize, Serialize};

pub const PLAYGROUND_HEADER_LEN: usize = 5;
pub const PLAYGROUND_MAX_JSON: usize = 16 * 1024;
pub const PLAYGROUND_MAX_JPEG: usize = 2 * 1024 * 1024;
pub const PLAYGROUND_MAX_TEXT: usize = 1024;
pub const PLAYGROUND_MAX_CHORD: usize = 64;

const KIND_STATE: u8 = 1;
const KIND_FRAME: u8 = 2;
const KIND_ERROR: u8 = 3;
const KIND_CONTROL: u8 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaygroundButton {
    Left,
    Right,
    Middle,
}

/// What the owner does while in control. Positions are pixels of the frame the owner sees.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlaygroundInput {
    PointerMove {
        x: f64,
        y: f64,
    },
    PointerDown {
        x: f64,
        y: f64,
        button: PlaygroundButton,
    },
    PointerUp {
        x: f64,
        y: f64,
        button: PlaygroundButton,
    },
    Scroll {
        x: f64,
        y: f64,
        dx: i32,
        dy: i32,
    },
    Key {
        chord: String,
    },
    Text {
        text: String,
    },
}

impl PlaygroundInput {
    pub fn validate(&self) -> Result<()> {
        let finite = |values: &[f64]| values.iter().all(|value| value.is_finite());
        match self {
            Self::PointerMove { x, y }
            | Self::PointerDown { x, y, .. }
            | Self::PointerUp { x, y, .. }
            | Self::Scroll { x, y, .. }
                if !finite(&[*x, *y]) =>
            {
                Err(ProtocolError::Invalid("playground input position"))
            }
            Self::Key { chord } if chord.is_empty() || chord.len() > PLAYGROUND_MAX_CHORD => {
                Err(ProtocolError::Invalid("playground key chord"))
            }
            Self::Text { text } if text.is_empty() || text.len() > PLAYGROUND_MAX_TEXT => {
                Err(ProtocolError::Invalid("playground text"))
            }
            _ => Ok(()),
        }
    }
}

/// Viewer to device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlaygroundControl {
    TakeOver,
    HandBack,
    Stop,
    Input { event: PlaygroundInput },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaygroundFrameKind {
    Browser,
    Computer,
}

/// How the device shows the bot's screen, so the viewer can hide controls it cannot use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaygroundDisplay {
    Virtual,
    Session,
    None,
}

/// The device's state of one bot's playground; sent on attach and whenever it changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaygroundState {
    pub taken_over: bool,
    /// No worker runs for the bot and no frame exists yet.
    pub asleep: bool,
    pub display: PlaygroundDisplay,
    /// Kind of the latest frame; browser frames are watch only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<PlaygroundFrameKind>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaygroundFrameMeta {
    pub width: u32,
    pub height: u32,
    pub kind: PlaygroundFrameKind,
    pub caption: String,
    /// Age of the picture when the device sent it.
    pub age_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<InteractionObservation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlaygroundFrame {
    pub meta: PlaygroundFrameMeta,
    pub jpeg: Vec<u8>,
}

/// Device to viewer.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaygroundEvent {
    State(PlaygroundState),
    Frame(PlaygroundFrame),
    Error { code: String, message: String },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorBody {
    code: String,
    message: String,
}

fn wrap(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(PLAYGROUND_HEADER_LEN + payload.len());
    bytes.push(kind);
    bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn json_payload<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let payload = serde_json::to_vec(value)
        .map_err(|_| ProtocolError::Invalid("playground message encoding"))?;
    if payload.len() > PLAYGROUND_MAX_JSON {
        return Err(ProtocolError::Invalid("playground message size"));
    }
    Ok(payload)
}

fn from_json<T: for<'de> Deserialize<'de>>(payload: &[u8]) -> Result<T> {
    serde_json::from_slice(payload).map_err(|_| ProtocolError::Invalid("playground message"))
}

impl PlaygroundControl {
    pub fn encode(&self) -> Result<Vec<u8>> {
        if let Self::Input { event } = self {
            event.validate()?;
        }
        Ok(wrap(KIND_CONTROL, &json_payload(self)?))
    }
}

impl PlaygroundEvent {
    pub fn encode(&self) -> Result<Vec<u8>> {
        match self {
            Self::State(state) => Ok(wrap(KIND_STATE, &json_payload(state)?)),
            Self::Error { code, message } => Ok(wrap(
                KIND_ERROR,
                &json_payload(&ErrorBody {
                    code: code.clone(),
                    message: message.clone(),
                })?,
            )),
            Self::Frame(frame) => {
                if frame.jpeg.is_empty() || frame.jpeg.len() > PLAYGROUND_MAX_JPEG {
                    return Err(ProtocolError::Invalid("playground frame size"));
                }
                let meta = json_payload(&frame.meta)?;
                let mut payload = Vec::with_capacity(2 + meta.len() + frame.jpeg.len());
                payload.extend_from_slice(&(meta.len() as u16).to_be_bytes());
                payload.extend_from_slice(&meta);
                payload.extend_from_slice(&frame.jpeg);
                Ok(wrap(KIND_FRAME, &payload))
            }
        }
    }
}

/// Reassembles messages from the chunks of a tunnel stream. Any malformed input is an error and
/// the stream must be closed; the decoder never allocates beyond the size caps.
#[derive(Default)]
pub struct PlaygroundDecoder {
    buffer: Vec<u8>,
}

impl PlaygroundDecoder {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    fn next_raw(&mut self) -> Result<Option<(u8, Vec<u8>)>> {
        if self.buffer.len() < PLAYGROUND_HEADER_LEN {
            return Ok(None);
        }
        let kind = self.buffer[0];
        let length = u32::from_be_bytes([
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
            self.buffer[4],
        ]) as usize;
        let limit = match kind {
            KIND_FRAME => 2 + PLAYGROUND_MAX_JSON + PLAYGROUND_MAX_JPEG,
            KIND_STATE | KIND_ERROR | KIND_CONTROL => PLAYGROUND_MAX_JSON,
            _ => return Err(ProtocolError::Invalid("playground message kind")),
        };
        if length > limit {
            return Err(ProtocolError::Invalid("playground message size"));
        }
        if self.buffer.len() < PLAYGROUND_HEADER_LEN + length {
            return Ok(None);
        }
        let payload = self.buffer[PLAYGROUND_HEADER_LEN..PLAYGROUND_HEADER_LEN + length].to_vec();
        self.buffer.drain(..PLAYGROUND_HEADER_LEN + length);
        Ok(Some((kind, payload)))
    }

    /// The next message the device sent, for the viewer.
    pub fn next_event(&mut self) -> Result<Option<PlaygroundEvent>> {
        let Some((kind, payload)) = self.next_raw()? else {
            return Ok(None);
        };
        Ok(Some(match kind {
            KIND_STATE => PlaygroundEvent::State(from_json(&payload)?),
            KIND_ERROR => {
                let body: ErrorBody = from_json(&payload)?;
                PlaygroundEvent::Error {
                    code: body.code,
                    message: body.message,
                }
            }
            KIND_FRAME => {
                if payload.len() < 2 {
                    return Err(ProtocolError::Invalid("playground frame"));
                }
                let meta_len = u16::from_be_bytes([payload[0], payload[1]]) as usize;
                if meta_len == 0 || meta_len > PLAYGROUND_MAX_JSON || payload.len() <= 2 + meta_len
                {
                    return Err(ProtocolError::Invalid("playground frame"));
                }
                let meta = from_json(&payload[2..2 + meta_len])?;
                let jpeg = payload[2 + meta_len..].to_vec();
                if jpeg.len() > PLAYGROUND_MAX_JPEG {
                    return Err(ProtocolError::Invalid("playground frame size"));
                }
                PlaygroundEvent::Frame(PlaygroundFrame { meta, jpeg })
            }
            _ => return Err(ProtocolError::Invalid("playground message kind")),
        }))
    }

    /// The next message the viewer sent, for the device.
    pub fn next_control(&mut self) -> Result<Option<PlaygroundControl>> {
        let Some((kind, payload)) = self.next_raw()? else {
            return Ok(None);
        };
        if kind != KIND_CONTROL {
            return Err(ProtocolError::Invalid("playground message kind"));
        }
        let control: PlaygroundControl = from_json(&payload)?;
        if let PlaygroundControl::Input { event } = &control {
            event.validate()?;
        }
        Ok(Some(control))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> PlaygroundState {
        PlaygroundState {
            taken_over: true,
            asleep: false,
            display: PlaygroundDisplay::Virtual,
            kind: Some(PlaygroundFrameKind::Computer),
        }
    }

    fn frame(jpeg: Vec<u8>) -> PlaygroundEvent {
        PlaygroundEvent::Frame(PlaygroundFrame {
            meta: PlaygroundFrameMeta {
                width: 1280,
                height: 800,
                kind: PlaygroundFrameKind::Computer,
                caption: "Desktop".into(),
                age_ms: 12,
                observation: None,
            },
            jpeg,
        })
    }

    #[test]
    fn events_survive_any_chunking() {
        let events = vec![
            PlaygroundEvent::State(state()),
            frame(vec![0xff, 0xd8, 1, 2, 3]),
            PlaygroundEvent::Error {
                code: "unauthorized".into(),
                message: "Only the owner".into(),
            },
        ];
        let bytes: Vec<u8> = events.iter().flat_map(|e| e.encode().unwrap()).collect();
        for chunk in [1usize, 3, 7, bytes.len()] {
            let mut decoder = PlaygroundDecoder::default();
            let mut decoded = Vec::new();
            for piece in bytes.chunks(chunk) {
                decoder.push(piece);
                while let Some(event) = decoder.next_event().unwrap() {
                    decoded.push(event);
                }
            }
            assert_eq!(decoded, events, "chunk size {chunk}");
        }
    }

    #[test]
    fn observations_round_trip_with_the_frame_and_legacy_frames_stay_unchanged() {
        let PlaygroundEvent::Frame(mut picture) = frame(vec![0xff, 0xd8, 1]) else {
            unreachable!()
        };
        let legacy = serde_json::to_value(&picture.meta).unwrap();
        assert!(legacy.get("observation").is_none());
        let decoded: PlaygroundFrameMeta = serde_json::from_value(legacy).unwrap();
        assert!(decoded.observation.is_none());
        picture.meta.observation = Some(InteractionObservation {
            captured_at_ms: 1000,
            width: 2560,
            height: 1600,
            pointer: Some(flow_like_types_contracts::interaction::InteractionPointer {
                position: flow_like_types_contracts::interaction::InteractionPoint {
                    x: 0.25,
                    y: 0.75,
                },
                source: flow_like_types_contracts::interaction::PointerSource::Observed,
            }),
            focus: None,
            action: None,
            mapping: None,
        });
        let event = PlaygroundEvent::Frame(picture);
        let mut decoder = PlaygroundDecoder::default();
        for chunk in event.encode().unwrap().chunks(3) {
            decoder.push(chunk);
        }
        assert_eq!(decoder.next_event().unwrap(), Some(event));
    }

    #[test]
    fn controls_round_trip_and_inputs_are_checked() {
        let controls = vec![
            PlaygroundControl::TakeOver,
            PlaygroundControl::HandBack,
            PlaygroundControl::Stop,
            PlaygroundControl::Input {
                event: PlaygroundInput::PointerDown {
                    x: 10.5,
                    y: 3.0,
                    button: PlaygroundButton::Left,
                },
            },
            PlaygroundControl::Input {
                event: PlaygroundInput::Key {
                    chord: "Control+c".into(),
                },
            },
        ];
        let mut decoder = PlaygroundDecoder::default();
        for control in &controls {
            decoder.push(&control.encode().unwrap());
        }
        for control in &controls {
            assert_eq!(decoder.next_control().unwrap().as_ref(), Some(control));
        }
        assert_eq!(decoder.next_control().unwrap(), None);

        let bad = [
            PlaygroundInput::PointerMove {
                x: f64::NAN,
                y: 0.0,
            },
            PlaygroundInput::Key {
                chord: String::new(),
            },
            PlaygroundInput::Key {
                chord: "a".repeat(PLAYGROUND_MAX_CHORD + 1),
            },
            PlaygroundInput::Text {
                text: "x".repeat(PLAYGROUND_MAX_TEXT + 1),
            },
        ];
        for event in bad {
            assert!(PlaygroundControl::Input { event }.encode().is_err());
        }
    }

    #[test]
    fn input_wire_shape_has_a_type_tag() {
        let event: PlaygroundInput =
            serde_json::from_str(r#"{"type":"scroll","x":1,"y":2,"dx":0,"dy":-3}"#).unwrap();
        assert_eq!(
            event,
            PlaygroundInput::Scroll {
                x: 1.0,
                y: 2.0,
                dx: 0,
                dy: -3
            }
        );
    }

    #[test]
    fn sizes_are_capped_both_ways() {
        assert!(frame(vec![0; PLAYGROUND_MAX_JPEG + 1]).encode().is_err());
        assert!(frame(Vec::new()).encode().is_err());
        let mut decoder = PlaygroundDecoder::default();
        let mut header = vec![KIND_STATE];
        header.extend_from_slice(&((PLAYGROUND_MAX_JSON + 1) as u32).to_be_bytes());
        decoder.push(&header);
        assert!(decoder.next_event().is_err());

        let mut decoder = PlaygroundDecoder::default();
        let mut header = vec![KIND_FRAME];
        header.extend_from_slice(&u32::MAX.to_be_bytes());
        decoder.push(&header);
        assert!(decoder.next_event().is_err());
    }

    #[test]
    fn unknown_or_misdirected_messages_are_errors() {
        let mut decoder = PlaygroundDecoder::default();
        decoder.push(&wrap(99, b"{}"));
        assert!(decoder.next_event().is_err());

        let mut decoder = PlaygroundDecoder::default();
        decoder.push(&wrap(KIND_STATE, b"{}"));
        assert!(decoder.next_control().is_err());

        let mut decoder = PlaygroundDecoder::default();
        decoder.push(&wrap(KIND_CONTROL, br#"{"type":"explode"}"#));
        assert!(decoder.next_control().is_err());

        let mut decoder = PlaygroundDecoder::default();
        decoder.push(&wrap(KIND_FRAME, &[0, 5, b'{']));
        assert!(decoder.next_event().is_err());
    }

    #[test]
    fn a_playground_open_names_only_a_bot() {
        let open = |bot: &str, service: &str| crate::TunnelOpen {
            placement_id: String::new(),
            service_id: service.into(),
            mode: crate::TunnelMode::Tcp,
            target: crate::TunnelTarget::BotPlayground {
                bot_id: bot.into(),
                observations: false,
            },
        };
        assert!(open("bot_1", "").validate().is_ok());
        assert!(open("", "").validate().is_err());
        assert!(open("bad id", "").validate().is_err());
        assert!(open("bot_1", "svc").validate().is_err());
        assert_eq!(
            serde_json::to_value(open("bot_1", "")).unwrap(),
            serde_json::json!({"target":{"bot_playground":{"bot_id":"bot_1"}}})
        );
    }
}

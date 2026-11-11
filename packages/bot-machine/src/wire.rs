//! The JSON lines the agent and a playground worker exchange: one request per line on the
//! worker's stdin, one response per line on its stdout. Pictures travel as base64.

use flow_like_bot_tools::browser::{BrowserStep, PageState, ScrollTo};
use flow_like_bot_tools::computer::{ComputerAction, PointerButton, ScrollDirection};
use flow_like_bot_tools::error::{ToolError, code};
use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like_types::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(default)]
    pub ok: Option<Value>,
    #[serde(default)]
    pub err: Option<WireError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireError {
    #[serde(default)]
    pub code: Option<String>,
    pub message: String,
}

const KNOWN_CODES: &[&str] = &[
    code::TAKEN_OVER,
    code::NOT_SUPPORTED,
    code::NO_DISPLAY,
    code::DISPLAY_BUSY,
    code::PERMISSION_MISSING,
    code::STALE_PAGE,
    code::NOT_FOUND,
    code::INVALID_ARGS,
    code::CANCELLED,
];

impl WireError {
    pub fn of(error: &flow_like_types::Error) -> Self {
        match error.downcast_ref::<ToolError>() {
            Some(tool) => Self {
                code: Some(tool.code.to_string()),
                message: tool.message.clone(),
            },
            None => Self {
                code: None,
                message: format!("{error:#}"),
            },
        }
    }

    pub fn into_error(self) -> flow_like_types::Error {
        let known = self
            .code
            .as_deref()
            .and_then(|wire| KNOWN_CODES.iter().copied().find(|known| *known == wire));
        match known {
            Some(code) => flow_like_types::Error::new(ToolError::new(code, self.message)),
            None => anyhow!("{}", self.message),
        }
    }
}

pub fn encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn decode(text: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(text)
        .map_err(|e| anyhow!("Playground sent a picture that is not base64: {e}"))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WirePage {
    pub url: String,
    pub title: String,
}

impl From<PageState> for WirePage {
    fn from(page: PageState) -> Self {
        Self {
            url: page.url,
            title: page.title,
        }
    }
}

impl From<WirePage> for PageState {
    fn from(page: WirePage) -> Self {
        Self {
            url: page.url,
            title: page.title,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireScroll {
    Up,
    Down,
    Top,
    Bottom,
    IntoView,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireStep {
    Click {
        reference: String,
    },
    Type {
        reference: String,
        text: String,
    },
    Select {
        reference: String,
        value: String,
    },
    Press {
        reference: Option<String>,
        key: String,
    },
    Scroll {
        reference: Option<String>,
        direction: WireScroll,
    },
}

impl From<&BrowserStep> for WireStep {
    fn from(step: &BrowserStep) -> Self {
        match step {
            BrowserStep::Click { reference } => Self::Click {
                reference: reference.clone(),
            },
            BrowserStep::Type { reference, text } => Self::Type {
                reference: reference.clone(),
                text: text.clone(),
            },
            BrowserStep::Select { reference, value } => Self::Select {
                reference: reference.clone(),
                value: value.clone(),
            },
            BrowserStep::Press { reference, key } => Self::Press {
                reference: reference.clone(),
                key: key.clone(),
            },
            BrowserStep::Scroll {
                reference,
                direction,
            } => Self::Scroll {
                reference: reference.clone(),
                direction: match direction {
                    ScrollTo::Up => WireScroll::Up,
                    ScrollTo::Down => WireScroll::Down,
                    ScrollTo::Top => WireScroll::Top,
                    ScrollTo::Bottom => WireScroll::Bottom,
                    ScrollTo::IntoView => WireScroll::IntoView,
                },
            },
        }
    }
}

impl From<WireStep> for BrowserStep {
    fn from(step: WireStep) -> Self {
        match step {
            WireStep::Click { reference } => Self::Click { reference },
            WireStep::Type { reference, text } => Self::Type { reference, text },
            WireStep::Select { reference, value } => Self::Select { reference, value },
            WireStep::Press { reference, key } => Self::Press { reference, key },
            WireStep::Scroll {
                reference,
                direction,
            } => Self::Scroll {
                reference,
                direction: match direction {
                    WireScroll::Up => ScrollTo::Up,
                    WireScroll::Down => ScrollTo::Down,
                    WireScroll::Top => ScrollTo::Top,
                    WireScroll::Bottom => ScrollTo::Bottom,
                    WireScroll::IntoView => ScrollTo::IntoView,
                },
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WireAction {
    Click {
        x: f64,
        y: f64,
        button: PointerButton,
        count: u32,
    },
    Move {
        x: f64,
        y: f64,
    },
    Type {
        text: String,
    },
    Key {
        chord: String,
    },
    Scroll {
        x: f64,
        y: f64,
        direction: WireDirection,
        ticks: u32,
    },
}

impl From<&ComputerAction> for WireAction {
    fn from(action: &ComputerAction) -> Self {
        match action {
            ComputerAction::Click {
                x,
                y,
                button,
                count,
            } => Self::Click {
                x: *x,
                y: *y,
                button: *button,
                count: *count,
            },
            ComputerAction::Move { x, y } => Self::Move { x: *x, y: *y },
            ComputerAction::Type { text } => Self::Type { text: text.clone() },
            ComputerAction::Key { chord } => Self::Key {
                chord: chord.clone(),
            },
            ComputerAction::Scroll {
                x,
                y,
                direction,
                ticks,
            } => Self::Scroll {
                x: *x,
                y: *y,
                direction: match direction {
                    ScrollDirection::Up => WireDirection::Up,
                    ScrollDirection::Down => WireDirection::Down,
                    ScrollDirection::Left => WireDirection::Left,
                    ScrollDirection::Right => WireDirection::Right,
                },
                ticks: *ticks,
            },
        }
    }
}

impl From<WireAction> for ComputerAction {
    fn from(action: WireAction) -> Self {
        match action {
            WireAction::Click {
                x,
                y,
                button,
                count,
            } => Self::Click {
                x,
                y,
                button,
                count,
            },
            WireAction::Move { x, y } => Self::Move { x, y },
            WireAction::Type { text } => Self::Type { text },
            WireAction::Key { chord } => Self::Key { chord },
            WireAction::Scroll {
                x,
                y,
                direction,
                ticks,
            } => Self::Scroll {
                x,
                y,
                direction: match direction {
                    WireDirection::Up => ScrollDirection::Up,
                    WireDirection::Down => ScrollDirection::Down,
                    WireDirection::Left => ScrollDirection::Left,
                    WireDirection::Right => ScrollDirection::Right,
                },
                ticks,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coded_errors_keep_their_code_across_the_wire_and_others_become_plain() {
        let coded = flow_like_types::Error::new(ToolError::new(code::TAKEN_OVER, "owner is here"));
        let back = WireError::of(&coded).into_error();
        assert_eq!(ToolError::from(back).code, code::TAKEN_OVER);
        let plain = WireError::of(&anyhow!("boom")).into_error();
        let tool = ToolError::from(plain);
        assert_eq!((tool.code, tool.message.as_str()), (code::FAILED, "boom"));
        let unknown = WireError {
            code: Some("made_up".into()),
            message: "x".into(),
        };
        assert_eq!(ToolError::from(unknown.into_error()).code, code::FAILED);
    }

    #[test]
    fn steps_and_actions_survive_a_json_round_trip() {
        let step = BrowserStep::Scroll {
            reference: None,
            direction: ScrollTo::IntoView,
        };
        let json = serde_json::to_value(WireStep::from(&step)).unwrap();
        let back: BrowserStep = serde_json::from_value::<WireStep>(json).unwrap().into();
        assert_eq!(back, step);
        let action = ComputerAction::Scroll {
            x: 1.0,
            y: 2.0,
            direction: ScrollDirection::Left,
            ticks: 3,
        };
        let json = serde_json::to_value(WireAction::from(&action)).unwrap();
        let back: ComputerAction = serde_json::from_value::<WireAction>(json).unwrap().into();
        assert_eq!(back, action);
    }
}

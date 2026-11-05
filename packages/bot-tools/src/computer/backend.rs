use async_trait::async_trait;
use flow_like_types::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{ToolError, code};

/// A picture of the screen. Coordinates in `computer_act` are pixels of the latest one.
#[derive(Debug, Clone)]
pub struct Screen {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// What the host wants the model to know with this picture (the owner used the computer).
    pub note: Option<String>,
}

impl Screen {
    pub fn new(png: Vec<u8>, width: u32, height: u32) -> Self {
        Self {
            png,
            width,
            height,
            note: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSummary {
    pub id: String,
    pub title: String,
    pub app: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub focused: bool,
    pub minimized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WindowOp {
    Focus,
    Close,
    Minimize,
    Maximize,
    Move,
    Resize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowBounds {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

pub fn not_supported(what: &str) -> flow_like_types::Error {
    flow_like_types::Error::new(ToolError::new(
        code::NOT_SUPPORTED,
        format!("This computer cannot {what}"),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PointerButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ComputerAction {
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
        direction: ScrollDirection,
        ticks: u32,
    },
}

/// The screen, mouse and keyboard of the machine the bot runs on.
#[async_trait]
pub trait ComputerBackend: Send + Sync {
    async fn screenshot(&self) -> Result<Screen>;

    /// Performs the action against the latest screenshot and says what it did.
    async fn act(&self, action: &ComputerAction) -> Result<String>;

    /// A picture of one window (an id or part of a title from `windows`); coordinates of
    /// `act` are pixels of it until the next screenshot.
    async fn screenshot_window(&self, _window: &str) -> Result<Screen> {
        Err(not_supported("capture a single window"))
    }

    async fn windows(&self) -> Result<Vec<WindowSummary>> {
        Err(not_supported("list its windows"))
    }

    async fn window(&self, _window: &str, _op: WindowOp, _bounds: WindowBounds) -> Result<String> {
        Err(not_supported("manage windows"))
    }

    /// Starts a program on the computer's display; `args` are passed as they are, never through a shell.
    async fn launch(&self, _program: &str, _args: &[String]) -> Result<String> {
        Err(not_supported("start programs"))
    }
}

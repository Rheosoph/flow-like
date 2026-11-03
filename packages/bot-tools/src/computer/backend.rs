use async_trait::async_trait;
use flow_like_types::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A picture of the screen. Coordinates in `computer_act` are pixels of the latest one.
#[derive(Debug, Clone)]
pub struct Screen {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
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
}

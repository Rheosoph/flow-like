//! `computer_*`: the bot sees the screen and uses the mouse and keyboard (capability
//! `computer_use`). The host implements [`ComputerBackend`] on the nodes' desktop input; the
//! spec makes every call ask the owner unless a standing grant (`computer:session`) covers it.

mod backend;
#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::time::Duration;

use flow_like_types::Bytes;
use flow_like_types::tokio;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::outcome::{ToolImage, ToolOutput};
use crate::secret_text::{Scrubber, fill_secrets};
use crate::spec::{Approval, ToolSpec, schema_of};

pub use backend::{ComputerAction, ComputerBackend, PointerButton, Screen, ScrollDirection};

pub const COMPUTER_SCREENSHOT: &str = "computer_screenshot";
pub const COMPUTER_ACT: &str = "computer_act";
pub const COMPUTER_WAIT: &str = "computer_wait";

const MAX_WAIT_MS: u64 = 10_000;
const MAX_TICKS: u32 = 30;
const DEFAULT_TICKS: u32 = 5;
const MAX_TYPED_CHARS: usize = 4_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComputerScreenshotArgs {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComputerActionKind {
    Click,
    DoubleClick,
    Type,
    Key,
    Scroll,
    Move,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComputerActArgs {
    pub action: ComputerActionKind,
    /// Horizontal pixel of the latest screenshot (click, double_click, move, scroll).
    #[serde(default)]
    pub x: Option<f64>,
    /// Vertical pixel of the latest screenshot.
    #[serde(default)]
    pub y: Option<f64>,
    /// `type`: the text to type, `{{secret:NAME}}` for a stored secret (its value is filled in
    /// and never shown). `key`: a chord such as Enter, Control+c or Meta+Shift+4. `scroll`: up,
    /// down, left or right.
    #[serde(default)]
    pub text: Option<String>,
    /// Mouse button for click (default left).
    #[serde(default)]
    pub button: Option<PointerButton>,
    /// Scroll ticks (default 5, at most 30).
    #[serde(default)]
    pub amount: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComputerWaitArgs {
    /// Milliseconds to wait, at most 10000.
    pub ms: u64,
}

pub fn specs() -> Vec<ToolSpec> {
    let computer = |name, description, approval, mutating, input_schema| ToolSpec {
        name,
        description,
        capability: Some("computer_use"),
        module: "computer",
        approval,
        approval_args: &[],
        mutating,
        input_schema,
    };
    vec![
        computer(
            COMPUTER_SCREENSHOT,
            "Look at the computer's screen. Coordinates for computer_act are pixels of this picture.",
            Approval::Ask,
            false,
            schema_of::<ComputerScreenshotArgs>(),
        ),
        computer(
            COMPUTER_ACT,
            "Click, double-click, move the pointer, type, press a key or scroll on the computer, then see the result. Use the latest screenshot's pixels. The owner approves unless they gave you the session.",
            Approval::Ask,
            true,
            schema_of::<ComputerActArgs>(),
        ),
        computer(
            COMPUTER_WAIT,
            "Wait for the screen to change, then take a screenshot.",
            Approval::None,
            false,
            schema_of::<ComputerWaitArgs>(),
        ),
    ]
}

fn backend_of(env: &dyn ToolEnv) -> ToolResult<std::sync::Arc<dyn ComputerBackend>> {
    env.computer().ok_or_else(|| {
        ToolError::new(
            code::NOT_YET,
            "This host cannot operate a computer: it runs on the desktop and on device bots, in an interactive session",
        )
    })
}

fn screen_output(screen: Screen, label: &str, result: serde_json::Value) -> ToolOutput {
    let mut output = ToolOutput::new(result);
    output.images.push(ToolImage {
        label: label.to_string(),
        mime: "image/png".into(),
        data: Bytes::from(screen.png),
    });
    output
}

fn size_json(screen: &Screen) -> serde_json::Value {
    json!({ "width": screen.width, "height": screen.height })
}

pub async fn computer_screenshot(
    env: &dyn ToolEnv,
    _args: ComputerScreenshotArgs,
) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    let screen = backend.screenshot().await.map_err(ToolError::from)?;
    let size = size_json(&screen);
    Ok(screen_output(
        screen,
        "Screenshot of the computer",
        json!({ "screen": size }),
    ))
}

fn point(args: &ComputerActArgs) -> ToolResult<(f64, f64)> {
    match (args.x, args.y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 => Ok((x, y)),
        _ => Err(ToolError::invalid(
            "This action needs 'x' and 'y', pixels of the latest screenshot",
        )),
    }
}

fn scroll_direction(text: Option<&str>) -> ToolResult<ScrollDirection> {
    match text.map(|t| t.trim().to_ascii_lowercase()).as_deref() {
        Some("up") => Ok(ScrollDirection::Up),
        Some("down") => Ok(ScrollDirection::Down),
        Some("left") => Ok(ScrollDirection::Left),
        Some("right") => Ok(ScrollDirection::Right),
        _ => Err(ToolError::invalid(
            "Scroll 'text' is up, down, left or right",
        )),
    }
}

async fn build_action(
    env: &dyn ToolEnv,
    args: &ComputerActArgs,
    scrubber: &mut Scrubber,
    used: &mut BTreeSet<String>,
) -> ToolResult<ComputerAction> {
    let button = args.button.unwrap_or(PointerButton::Left);
    Ok(match args.action {
        ComputerActionKind::Click | ComputerActionKind::DoubleClick => {
            let (x, y) = point(args)?;
            ComputerAction::Click {
                x,
                y,
                button,
                count: if args.action == ComputerActionKind::DoubleClick {
                    2
                } else {
                    1
                },
            }
        }
        ComputerActionKind::Move => {
            let (x, y) = point(args)?;
            ComputerAction::Move { x, y }
        }
        ComputerActionKind::Type => {
            let text = args
                .text
                .clone()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| ToolError::invalid("Typing needs 'text'"))?;
            let text = fill_secrets(env, &text, scrubber, used).await?;
            if text.chars().count() > MAX_TYPED_CHARS {
                return Err(ToolError::invalid(format!(
                    "Type at most {MAX_TYPED_CHARS} characters per call"
                )));
            }
            ComputerAction::Type { text }
        }
        ComputerActionKind::Key => ComputerAction::Key {
            chord: args
                .text
                .clone()
                .filter(|chord| !chord.trim().is_empty())
                .ok_or_else(|| ToolError::invalid("A key press needs 'text', such as Enter"))?,
        },
        ComputerActionKind::Scroll => {
            let (x, y) = point(args)?;
            ComputerAction::Scroll {
                x,
                y,
                direction: scroll_direction(args.text.as_deref())?,
                ticks: args.amount.unwrap_or(DEFAULT_TICKS).clamp(1, MAX_TICKS),
            }
        }
    })
}

pub async fn computer_act(env: &dyn ToolEnv, args: ComputerActArgs) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    let mut scrubber = Scrubber::default();
    let mut used = BTreeSet::new();
    let result = async {
        let action = build_action(env, &args, &mut scrubber, &mut used).await?;
        let done = backend.act(&action).await.map_err(ToolError::from)?;
        let screen = backend.screenshot().await.map_err(ToolError::from)?;
        let size = size_json(&screen);
        Ok(screen_output(
            screen,
            "The computer after the action",
            json!({ "did": done, "screen": size }),
        ))
    }
    .await;
    match result {
        Ok(mut output) => {
            scrubber.scrub_value(&mut output.result);
            Ok(output)
        }
        Err(error) => Err(scrubber.scrub_error(error)),
    }
}

pub async fn computer_wait(env: &dyn ToolEnv, args: ComputerWaitArgs) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    let ms = args.ms.min(MAX_WAIT_MS);
    let mut waited = 0;
    while waited < ms {
        if env.is_cancelled() {
            return Err(ToolError::new(code::CANCELLED, "The wait was cancelled"));
        }
        let step = (ms - waited).min(250);
        tokio::time::sleep(Duration::from_millis(step)).await;
        waited += step;
    }
    let screen = backend.screenshot().await.map_err(ToolError::from)?;
    let size = size_json(&screen);
    Ok(screen_output(
        screen,
        "The computer after waiting",
        json!({ "waited_ms": ms, "screen": size }),
    ))
}

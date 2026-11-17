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

pub use backend::{
    ComputerAction, ComputerBackend, DisplaySummary, PointerButton, ReadText, Screen,
    ScrollDirection, TextMethod, TextSource, WindowBounds, WindowOp, WindowSummary, not_supported,
};

pub const COMPUTER_SCREENSHOT: &str = "computer_screenshot";
pub const COMPUTER_ACT: &str = "computer_act";
pub const COMPUTER_WAIT: &str = "computer_wait";
pub const COMPUTER_WINDOWS: &str = "computer_windows";
pub const COMPUTER_WINDOW: &str = "computer_window";
pub const COMPUTER_LAUNCH: &str = "computer_launch";
pub const COMPUTER_READ_TEXT: &str = "computer_read_text";

const MAX_WAIT_MS: u64 = 10_000;
const MAX_TICKS: u32 = 30;
const DEFAULT_TICKS: u32 = 5;
const MAX_TYPED_CHARS: usize = 4_000;
const MAX_PASTED_CHARS: usize = 100_000;
const DEFAULT_READ_CHARS: usize = 20_000;
const MAX_READ_CHARS: usize = 100_000;
const MAX_LAUNCH_ARGS: usize = 32;
const MAX_LAUNCH_CHARS: usize = 2_000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComputerScreenshotArgs {
    /// A window id or part of a title from computer_windows: the picture shows only that window
    /// and computer_act coordinates are pixels of it. Omit for the whole screen.
    #[serde(default)]
    pub window: Option<String>,
    /// A display index from the 'displays' list of an earlier screenshot. Omit for the primary
    /// display. Ignored when 'window' is set.
    #[serde(default)]
    pub display: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComputerWindowsArgs {
    /// Also list windows on other Spaces or hidden. Default false.
    #[serde(default)]
    pub include_offscreen: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComputerWindowArgs {
    pub action: WindowOp,
    /// A window id or part of a title from computer_windows.
    pub window: String,
    /// move: the window's new left edge in screen pixels.
    #[serde(default)]
    pub x: Option<i32>,
    /// move: the window's new top edge in screen pixels.
    #[serde(default)]
    pub y: Option<i32>,
    /// resize: the new width in pixels.
    #[serde(default)]
    pub width: Option<i32>,
    /// resize: the new height in pixels.
    #[serde(default)]
    pub height: Option<i32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComputerLaunchArgs {
    /// The program to start: a name on the PATH or a full path. Never a shell command line.
    pub program: String,
    /// Arguments, one per entry, passed as they are.
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComputerActionKind {
    Click,
    DoubleClick,
    /// Selects a whole line or paragraph of text.
    TripleClick,
    Type,
    /// Inserts 'text' at once through the clipboard: use it for long text.
    Paste,
    /// Selects everything in the focused field, after clicking at 'x'/'y' when given.
    SelectAll,
    /// Presses at 'x'/'y', moves to 'to_x'/'to_y' and releases: selects a range of text or moves
    /// something.
    Drag,
    Key,
    Scroll,
    Move,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComputerActArgs {
    pub action: ComputerActionKind,
    /// Horizontal pixel of the latest screenshot (click, double_click, triple_click, move,
    /// scroll, the start of a drag; optional for select_all).
    #[serde(default)]
    pub x: Option<f64>,
    /// Vertical pixel of the latest screenshot.
    #[serde(default)]
    pub y: Option<f64>,
    /// Drag: where to release, a horizontal pixel of the latest screenshot.
    #[serde(default)]
    pub to_x: Option<f64>,
    /// Drag: where to release, a vertical pixel of the latest screenshot.
    #[serde(default)]
    pub to_y: Option<f64>,
    /// `type` and `paste`: the text, `{{secret:NAME}}` for a stored secret (its value is filled
    /// in and never shown). `key`: a chord such as Enter, Control+c or Meta+Shift+4. `scroll`:
    /// up, down, left or right.
    #[serde(default)]
    pub text: Option<String>,
    /// `paste`: replace the focused field's whole text instead of inserting at the caret.
    #[serde(default)]
    pub replace_all: bool,
    /// Mouse button for click (default left).
    #[serde(default)]
    pub button: Option<PointerButton>,
    /// Scroll ticks (default 5, at most 30).
    #[serde(default)]
    pub amount: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ComputerReadTextArgs {
    /// field (default): the focused text field's whole text. selection: only the selected text.
    #[serde(default)]
    pub source: TextSource,
    /// Click here first to focus the field: a horizontal pixel of the latest screenshot.
    #[serde(default)]
    pub x: Option<f64>,
    /// Click here first to focus the field: a vertical pixel of the latest screenshot.
    #[serde(default)]
    pub y: Option<f64>,
    /// Most characters to return (default 20000, at most 100000); longer text is cut.
    #[serde(default)]
    pub max_chars: Option<usize>,
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
            "Look at the computer's screen (the primary display, or another with 'display'; the result lists the displays), or one window of it with 'window'. Coordinates for computer_act are pixels of this picture. Fails naming the missing permission (screen_recording) when macOS has not granted it.",
            Approval::Ask,
            false,
            schema_of::<ComputerScreenshotArgs>(),
        ),
        computer(
            COMPUTER_ACT,
            "Click, double- or triple-click, drag, move the pointer, type, paste, select all, press a key or scroll on the computer, then see the result. Use the latest screenshot's pixels. Paste long text instead of typing it. The owner approves unless they gave you the session.",
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
        computer(
            COMPUTER_WINDOWS,
            "List the windows open on the computer: id, title, application, position, size and whether each is focused, minimized or onscreen. Windows on other Spaces and helper windows are left out unless 'include_offscreen' is true. Fails naming the missing permission (accessibility) when macOS has not granted it.",
            Approval::None,
            false,
            schema_of::<ComputerWindowsArgs>(),
        ),
        computer(
            COMPUTER_WINDOW,
            "Focus, close, minimize, maximize, move or resize one window; focusing a window on another Space switches to it. The owner approves unless they gave you the session.",
            Approval::Ask,
            true,
            schema_of::<ComputerWindowArgs>(),
        ),
        computer(
            COMPUTER_LAUNCH,
            "Start a program on the computer's display. The owner approves each program.",
            Approval::Ask,
            true,
            schema_of::<ComputerLaunchArgs>(),
        ),
        computer(
            COMPUTER_READ_TEXT,
            "Read the text of the focused text field, or the selected text, as text instead of from a screenshot: exact, and complete even where the field scrolls. Give 'x'/'y' to click into the field first. Reads through the app's accessibility where it can, else copies the text and restores the clipboard; copying a whole field selects its text. The owner approves unless they gave you the session.",
            Approval::Ask,
            false,
            schema_of::<ComputerReadTextArgs>(),
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

fn screen_output(screen: Screen, label: &str, mut result: serde_json::Value) -> ToolOutput {
    if let (Some(observation), Some(fields)) = (&screen.observation, result.as_object_mut()) {
        fields.insert("observation".into(), json!(observation));
    }
    if let (Some(note), Some(fields)) = (&screen.note, result.as_object_mut()) {
        fields.insert("note".into(), json!(note));
    }
    let mut output = ToolOutput::new(result);
    let mut annotated = ToolOutput::new(json!({}));
    if crate::interaction_image::append(&mut annotated, &screen.png, screen.observation.as_ref()) {
        output.result["feedback_legend"] = json!(crate::interaction_image::LEGEND);
    }
    output.images.push(ToolImage {
        label: label.to_string(),
        mime: "image/png".into(),
        data: Bytes::from(screen.png),
    });
    output.images.extend(annotated.images);
    output
}

fn size_json(screen: &Screen) -> serde_json::Value {
    json!({ "width": screen.width, "height": screen.height })
}

pub async fn computer_screenshot(
    env: &dyn ToolEnv,
    args: ComputerScreenshotArgs,
) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    let window = args
        .window
        .as_deref()
        .map(str::trim)
        .filter(|window| !window.is_empty());
    let (screen, label) = match window {
        Some(window) => (
            backend.screenshot_window(window).await,
            "Screenshot of the window",
        ),
        None => (
            backend.screenshot_display(args.display).await,
            "Screenshot of the computer",
        ),
    };
    let screen = screen.map_err(ToolError::from)?;
    let size = size_json(&screen);
    let mut result = json!({ "screen": size });
    if window.is_none() {
        let displays = backend.displays().await.unwrap_or_default();
        if displays.len() > 1 {
            result["displays"] = json!(displays);
            result["display"] = json!(args.display);
        }
    }
    Ok(screen_output(screen, label, result))
}

pub async fn computer_windows(
    env: &dyn ToolEnv,
    args: ComputerWindowsArgs,
) -> ToolResult<ToolOutput> {
    let windows = backend_of(env)?
        .windows_listed(args.include_offscreen)
        .await
        .map_err(ToolError::from)?;
    Ok(ToolOutput::new(
        json!({ "count": windows.len(), "windows": windows }),
    ))
}

fn window_query(window: &str) -> ToolResult<&str> {
    let window = window.trim();
    if window.is_empty() {
        return Err(ToolError::invalid(
            "'window' is a window id or part of a title from computer_windows",
        ));
    }
    Ok(window)
}

fn window_bounds(args: &ComputerWindowArgs) -> ToolResult<WindowBounds> {
    let bounds = WindowBounds {
        x: args.x,
        y: args.y,
        width: args.width,
        height: args.height,
    };
    match args.action {
        WindowOp::Move if bounds.x.is_none() && bounds.y.is_none() => Err(ToolError::invalid(
            "Moving a window needs 'x' and/or 'y', screen pixels",
        )),
        WindowOp::Resize if bounds.width.is_none() && bounds.height.is_none() => Err(
            ToolError::invalid("Resizing a window needs 'width' and/or 'height', pixels"),
        ),
        WindowOp::Resize
            if bounds.width.is_some_and(|w| w <= 0) || bounds.height.is_some_and(|h| h <= 0) =>
        {
            Err(ToolError::invalid(
                "A window's width and height are positive",
            ))
        }
        _ => Ok(bounds),
    }
}

pub async fn computer_window(
    env: &dyn ToolEnv,
    args: ComputerWindowArgs,
) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    let window = window_query(&args.window)?;
    let bounds = window_bounds(&args)?;
    let done = backend
        .window(window, args.action, bounds)
        .await
        .map_err(ToolError::from)?;
    Ok(ToolOutput::new(json!({ "did": done })))
}

/// The program's file name, lowercased: what a standing launch grant names.
pub fn launch_scope(program: &str) -> Option<String> {
    let name = program
        .trim()
        .rsplit(['/', '\\'])
        .next()?
        .trim()
        .to_ascii_lowercase();
    (!name.is_empty()).then_some(name)
}

fn launch_request(args: &ComputerLaunchArgs) -> ToolResult<()> {
    let too_long = |text: &String| text.chars().count() > MAX_LAUNCH_CHARS || text.contains('\0');
    if launch_scope(&args.program).is_none() {
        return Err(ToolError::invalid(
            "'program' names the program to start, such as firefox",
        ));
    }
    if args.args.len() > MAX_LAUNCH_ARGS {
        return Err(ToolError::invalid(format!(
            "Pass at most {MAX_LAUNCH_ARGS} arguments"
        )));
    }
    if too_long(&args.program) || args.args.iter().any(too_long) {
        return Err(ToolError::invalid(format!(
            "The program and each argument are at most {MAX_LAUNCH_CHARS} characters without NUL bytes"
        )));
    }
    Ok(())
}

pub async fn computer_launch(
    env: &dyn ToolEnv,
    args: ComputerLaunchArgs,
) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    launch_request(&args)?;
    let done = backend
        .launch(args.program.trim(), &args.args)
        .await
        .map_err(ToolError::from)?;
    Ok(ToolOutput::new(json!({ "did": done })))
}

fn pixel(x: Option<f64>, y: Option<f64>) -> Option<(f64, f64)> {
    match (x, y) {
        (Some(x), Some(y)) if x.is_finite() && y.is_finite() && x >= 0.0 && y >= 0.0 => {
            Some((x, y))
        }
        _ => None,
    }
}

fn point(args: &ComputerActArgs) -> ToolResult<(f64, f64)> {
    pixel(args.x, args.y).ok_or_else(|| {
        ToolError::invalid("This action needs 'x' and 'y', pixels of the latest screenshot")
    })
}

/// A point given only partly, or not as pixels, is a mistake; none at all is fine.
fn optional_point(x: Option<f64>, y: Option<f64>) -> ToolResult<Option<(f64, f64)>> {
    if x.is_none() && y.is_none() {
        return Ok(None);
    }
    pixel(x, y).map(Some).ok_or_else(|| {
        ToolError::invalid("Give both 'x' and 'y', pixels of the latest screenshot, or neither")
    })
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
        ComputerActionKind::Click
        | ComputerActionKind::DoubleClick
        | ComputerActionKind::TripleClick => {
            let (x, y) = point(args)?;
            ComputerAction::Click {
                x,
                y,
                button,
                count: match args.action {
                    ComputerActionKind::DoubleClick => 2,
                    ComputerActionKind::TripleClick => 3,
                    _ => 1,
                },
            }
        }
        ComputerActionKind::Paste => {
            let text = args
                .text
                .clone()
                .filter(|text| !text.is_empty())
                .ok_or_else(|| ToolError::invalid("Pasting needs 'text'"))?;
            let text = fill_secrets(env, &text, scrubber, used).await?;
            if text.chars().count() > MAX_PASTED_CHARS {
                return Err(ToolError::invalid(format!(
                    "Paste at most {MAX_PASTED_CHARS} characters per call"
                )));
            }
            ComputerAction::Paste {
                text,
                replace_all: args.replace_all,
            }
        }
        ComputerActionKind::SelectAll => ComputerAction::SelectAll {
            at: optional_point(args.x, args.y)?,
        },
        ComputerActionKind::Drag => ComputerAction::Drag {
            from: point(args)?,
            to: pixel(args.to_x, args.to_y).ok_or_else(|| {
                ToolError::invalid(
                    "A drag needs 'to_x' and 'to_y', where to release, pixels of the latest screenshot",
                )
            })?,
            button,
        },
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

/// Cuts `text` to `max` characters on a character boundary; true when it cut.
fn cut(text: &str, max: usize) -> (String, bool) {
    match text.char_indices().nth(max) {
        Some((end, _)) => (text[..end].to_string(), true),
        None => (text.to_string(), false),
    }
}

pub async fn computer_read_text(
    env: &dyn ToolEnv,
    args: ComputerReadTextArgs,
) -> ToolResult<ToolOutput> {
    let backend = backend_of(env)?;
    let at = optional_point(args.x, args.y)?;
    let max = args
        .max_chars
        .unwrap_or(DEFAULT_READ_CHARS)
        .clamp(1, MAX_READ_CHARS);
    let read = backend
        .read_text(args.source, at)
        .await
        .map_err(ToolError::from)?;
    let chars = read.text.chars().count();
    let (text, truncated) = cut(&read.text, max);
    let mut result = json!({
        "source": args.source,
        "method": read.method,
        "chars": chars,
        "text": text,
    });
    if truncated {
        result["truncated"] = json!(true);
        result["note"] = json!(format!(
            "Only the first {max} of {chars} characters; raise 'max_chars' (at most {MAX_READ_CHARS}) for more"
        ));
    }
    if chars == 0 {
        result["note"] = json!(match args.source {
            TextSource::Selection => "Nothing is selected",
            TextSource::Field => "The focused field is empty, or no text field has focus",
        });
    }
    Ok(ToolOutput::new(result))
}

fn preview(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let (shown, cut_off) = cut(&flat, 80);
    if cut_off {
        format!("{shown}…")
    } else {
        shown
    }
}

/// What the owner is asked before reading or pasting text; other computer calls keep the
/// generic question.
pub fn ask_message(tool: &str, args: &serde_json::Value) -> Option<String> {
    match tool {
        COMPUTER_READ_TEXT => Some(
            match args.get("source").and_then(serde_json::Value::as_str) {
                Some("selection") => "The bot wants to read the selected text on your screen.",
                _ => "The bot wants to read the text of the focused field on your screen.",
            }
            .to_string(),
        ),
        COMPUTER_ACT if args.get("action").and_then(serde_json::Value::as_str) == Some("paste") => {
            let text = args
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let count = text.chars().count();
            let how = if args.get("replace_all").and_then(serde_json::Value::as_bool) == Some(true)
            {
                "replace the focused field's text with"
            } else {
                "paste"
            };
            Some(format!(
                "The bot wants to {how} {count} characters: “{}”",
                preview(text)
            ))
        }
        _ => None,
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
    match backend.screenshot().await.map_err(ToolError::from) {
        Ok(screen) => {
            let size = size_json(&screen);
            Ok(screen_output(
                screen,
                "The computer after waiting",
                json!({ "waited_ms": ms, "screen": size }),
            ))
        }
        Err(error) if error.code == code::TAKEN_OVER => Ok(ToolOutput::new(json!({
            "waited_ms": ms,
            "taken_over": true,
            "message": error.message,
        }))),
        Err(error) => Err(error),
    }
}

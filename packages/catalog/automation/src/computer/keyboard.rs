use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

const KEY_NAMES: &str = "a single character or a named key: Enter, Tab, Escape, Backspace, Delete, Space, arrows (Up/Down/Left/Right), Home, End, PageUp, PageDown, Insert, CapsLock, NumLock, ScrollLock, PrintScreen, Pause, Help, F1–F24, Shift, Ctrl, Alt, Cmd/Meta/Super/Win, Numpad0–Numpad9, NumpadAdd, NumpadSubtract, NumpadMultiply, NumpadDivide, NumpadDecimal, VolumeUp, VolumeDown, VolumeMute, MediaPlayPause, MediaNext, MediaPrev. Keys the operating system cannot send fail with an error";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordModifier {
    Control,
    Shift,
    Alt,
    Meta,
}

/// A parsed shortcut such as `ctrl+shift+s`: modifiers held in order while `key` is clicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyChord {
    pub modifiers: Vec<ChordModifier>,
    pub key: String,
}

pub fn chord_modifier(name: &str) -> Option<ChordModifier> {
    match name.trim().to_lowercase().as_str() {
        "ctrl" | "control" | "ctl" => Some(ChordModifier::Control),
        "shift" => Some(ChordModifier::Shift),
        "alt" | "option" | "opt" => Some(ChordModifier::Alt),
        "cmd" | "command" | "meta" | "super" | "win" | "windows" => Some(ChordModifier::Meta),
        "primary" | "cmdorctrl" => Some(if cfg!(target_os = "macos") {
            ChordModifier::Meta
        } else {
            ChordModifier::Control
        }),
        _ => None,
    }
}

/// Parses `ctrl+shift+s`, `cmd+space`, `alt+F4` or `ctrl++`. The last segment is the key.
pub fn parse_key_chord(text: &str) -> flow_like_types::Result<KeyChord> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(flow_like_types::anyhow!("Key chord is empty"));
    }
    let (head, key) = if trimmed == "+" {
        ("", "+")
    } else if let Some(head) = trimmed.strip_suffix("++") {
        (head, "+")
    } else {
        match trimmed.rsplit_once('+') {
            Some((head, key)) => (head, key.trim()),
            None => ("", trimmed),
        }
    };
    if key.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Key chord '{}' has no key after the last '+'",
            text
        ));
    }
    let mut modifiers = Vec::new();
    if !head.trim().is_empty() {
        for part in head.split('+') {
            let modifier = chord_modifier(part).ok_or_else(|| {
                flow_like_types::anyhow!(
                    "'{}' in key chord '{}' is not a modifier; use ctrl, shift, alt, cmd/meta/super/win or primary",
                    part.trim(),
                    text
                )
            })?;
            if !modifiers.contains(&modifier) {
                modifiers.push(modifier);
            }
        }
    }
    let key = if key.len() == 1 && key.chars().all(|c| c.is_ascii_alphabetic()) {
        key.to_ascii_lowercase()
    } else {
        key.to_owned()
    };
    Ok(KeyChord { modifiers, key })
}

#[cfg(feature = "execute")]
fn function_key(number: u8) -> Option<enigo::Key> {
    use enigo::Key;
    Some(match number {
        1 => Key::F1,
        2 => Key::F2,
        3 => Key::F3,
        4 => Key::F4,
        5 => Key::F5,
        6 => Key::F6,
        7 => Key::F7,
        8 => Key::F8,
        9 => Key::F9,
        10 => Key::F10,
        11 => Key::F11,
        12 => Key::F12,
        13 => Key::F13,
        14 => Key::F14,
        15 => Key::F15,
        16 => Key::F16,
        17 => Key::F17,
        18 => Key::F18,
        19 => Key::F19,
        20 => Key::F20,
        #[cfg(not(target_os = "macos"))]
        21 => Key::F21,
        #[cfg(not(target_os = "macos"))]
        22 => Key::F22,
        #[cfg(not(target_os = "macos"))]
        23 => Key::F23,
        #[cfg(not(target_os = "macos"))]
        24 => Key::F24,
        _ => return None,
    })
}

#[cfg(feature = "execute")]
fn platform_key(name: &str) -> Option<enigo::Key> {
    #[cfg(not(target_os = "macos"))]
    {
        use enigo::Key;
        let key = match name {
            "insert" | "ins" => Key::Insert,
            "numlock" => Key::Numlock,
            "pause" | "break" => Key::Pause,
            "printscreen" | "print" | "printscr" | "prtsc" => Key::PrintScr,
            "mediastop" => Key::MediaStop,
            #[cfg(target_os = "windows")]
            "scrolllock" => Key::Scroll,
            #[cfg(target_os = "linux")]
            "scrolllock" => Key::ScrollLock,
            _ => return None,
        };
        Some(key)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = name;
        None
    }
}

#[cfg(feature = "execute")]
const PLATFORM_ONLY_KEYS: [&str; 15] = [
    "insert",
    "ins",
    "numlock",
    "pause",
    "break",
    "printscreen",
    "print",
    "printscr",
    "prtsc",
    "mediastop",
    "scrolllock",
    "f21",
    "f22",
    "f23",
    "f24",
];

/// Maps a key name (case-insensitive) or a single character to the key the OS sends.
#[cfg(feature = "execute")]
pub(crate) fn parse_key(name: &str) -> flow_like_types::Result<enigo::Key> {
    use enigo::Key;
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        return Ok(Key::Unicode(c));
    }
    let lower = name.trim().to_lowercase();
    let key = match lower.as_str() {
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "escape" | "esc" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" | "del" | "forwarddelete" => Key::Delete,
        "space" | "spacebar" => Key::Space,
        "up" | "arrowup" => Key::UpArrow,
        "down" | "arrowdown" => Key::DownArrow,
        "left" | "arrowleft" => Key::LeftArrow,
        "right" | "arrowright" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "capslock" => Key::CapsLock,
        "help" => Key::Help,
        "shift" => Key::Shift,
        "ctrl" | "control" => Key::Control,
        "alt" | "option" => Key::Alt,
        "meta" | "cmd" | "command" | "super" | "win" | "windows" => Key::Meta,
        "numpad0" => Key::Numpad0,
        "numpad1" => Key::Numpad1,
        "numpad2" => Key::Numpad2,
        "numpad3" => Key::Numpad3,
        "numpad4" => Key::Numpad4,
        "numpad5" => Key::Numpad5,
        "numpad6" => Key::Numpad6,
        "numpad7" => Key::Numpad7,
        "numpad8" => Key::Numpad8,
        "numpad9" => Key::Numpad9,
        "numpadadd" | "add" => Key::Add,
        "numpadsubtract" | "subtract" => Key::Subtract,
        "numpadmultiply" | "multiply" => Key::Multiply,
        "numpaddivide" | "divide" => Key::Divide,
        "numpaddecimal" | "decimal" => Key::Decimal,
        "volumeup" => Key::VolumeUp,
        "volumedown" => Key::VolumeDown,
        "volumemute" | "mute" => Key::VolumeMute,
        "mediaplaypause" | "playpause" => Key::MediaPlayPause,
        "medianext" | "medianexttrack" | "nexttrack" => Key::MediaNextTrack,
        "mediaprev" | "mediaprevious" | "mediaprevtrack" | "previoustrack" => {
            Key::MediaPrevTrack
        }
        other => {
            if let Some(key) = other
                .strip_prefix('f')
                .and_then(|number| number.parse::<u8>().ok())
                .and_then(function_key)
            {
                return Ok(key);
            }
            if let Some(key) = platform_key(other) {
                return Ok(key);
            }
            if PLATFORM_ONLY_KEYS.contains(&other) {
                return Err(flow_like_types::anyhow!(
                    "Key '{}' cannot be sent on {}",
                    name,
                    std::env::consts::OS
                ));
            }
            return Err(flow_like_types::anyhow!(
                "Unknown key '{}'. Use {}",
                name,
                KEY_NAMES
            ));
        }
    };
    Ok(key)
}

#[cfg(feature = "execute")]
fn chord_modifier_key(modifier: ChordModifier) -> enigo::Key {
    match modifier {
        ChordModifier::Control => enigo::Key::Control,
        ChordModifier::Shift => enigo::Key::Shift,
        ChordModifier::Alt => enigo::Key::Alt,
        ChordModifier::Meta => enigo::Key::Meta,
    }
}

#[cfg(feature = "execute")]
fn press_chord(
    input: &mut super::native::input::DesktopInput,
    modifiers: &[enigo::Key],
    key: enigo::Key,
) -> flow_like_types::Result<()> {
    use enigo::{Direction, Keyboard};
    let mut result = Ok(());
    let mut pressed = Vec::new();
    for modifier in modifiers {
        if let Err(error) = input.key(*modifier, Direction::Press) {
            result = Err(flow_like_types::anyhow!(
                "Failed to press modifier {:?}: {}",
                modifier,
                error
            ));
            break;
        }
        pressed.push(*modifier);
    }
    if result.is_ok()
        && let Err(error) = input.key(key, Direction::Click)
    {
        result = Err(flow_like_types::anyhow!(
            "Failed to press key {:?}: {}",
            key,
            error
        ));
    }
    for modifier in pressed.into_iter().rev() {
        if let Err(error) = input.key(modifier, Direction::Release)
            && result.is_ok()
        {
            result = Err(flow_like_types::anyhow!(
                "Failed to release modifier {:?}: {}",
                modifier,
                error
            ));
        }
    }
    result
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerKeyPressNode {}

impl ComputerKeyPressNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerKeyPressNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_key_press",
            "Key Press",
            "Presses a keyboard key or key combination",
            "Automation/Computer/Keyboard",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "keyPress");
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "key",
            "Key",
            &format!("Key to press (case-insensitive): {KEY_NAMES}"),
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "modifiers",
            "Modifiers",
            "Modifier keys to hold (comma-separated: ctrl,shift,alt,meta)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use enigo::{Direction, Keyboard};

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let key_str: String = context.evaluate_pin("key").await?;
        let modifiers: String = context.evaluate_pin("modifiers").await?;
        let key = parse_key(&key_str)?;

        let mut enigo = session.create_enigo(context).await?;
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            enigo.modifiers(&modifiers)?;
            enigo
                .key(key, Direction::Click)
                .map_err(|e| flow_like_types::anyhow!("Failed to press key {}: {}", key_str, e))?;

            Ok(())
        })
        .await??;

        session.apply_delay(context).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerKeyTypeNode {}

impl ComputerKeyTypeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerKeyTypeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_key_type",
            "Type Text",
            "Types text using the keyboard. The text is stored in the board; use Type Secret for passwords",
            "Automation/Computer/Keyboard",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "typeText");
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin("text", "Text", "Text to type", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use enigo::Keyboard;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let text: String = context.evaluate_pin("text").await?;

        let mut enigo = session.create_enigo(context).await?;
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            enigo
                .text(&text)
                .map_err(|e| flow_like_types::anyhow!("Failed to type text: {}", e))?;

            Ok(())
        })
        .await??;

        session.apply_delay(context).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerKeyChordNode {}

impl ComputerKeyChordNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerKeyChordNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_key_chord",
            "Key Chord",
            "Presses a keyboard shortcut written as text, such as ctrl+shift+s, cmd+space or alt+F4. Modifiers are released in reverse order even when a key fails",
            "Automation/Computer/Keyboard",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "keyChord");
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "chord",
            "Chord",
            &format!(
                "Modifiers joined with '+' followed by one key, e.g. ctrl+shift+s, cmd+space, alt+F4, ctrl++. Modifiers: ctrl, shift, alt/option, cmd/meta/super/win, primary (cmd on macOS, ctrl elsewhere). Key: {KEY_NAMES}"
            ),
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "repeat",
            "Repeat",
            "How many times to press the chord (1-100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let text: String = context.evaluate_pin("chord").await?;
        let repeat: i64 = context.evaluate_pin("repeat").await?;
        if !(1..=100).contains(&repeat) {
            return Err(flow_like_types::anyhow!(
                "Repeat must be between 1 and 100, got {}",
                repeat
            ));
        }
        let chord = parse_key_chord(&text)?;
        let modifiers: Vec<enigo::Key> = chord
            .modifiers
            .iter()
            .copied()
            .map(chord_modifier_key)
            .collect();
        let key = parse_key(&chord.key)?;

        let mut input = session.create_enigo(context).await?;
        let cancellation = context.get_cancellation_token();
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            for index in 0..repeat {
                crate::computer::mouse::check_cancellation(cancellation.as_ref())?;
                if index > 0 {
                    crate::computer::mouse::interruptible_sleep(40, cancellation.as_ref())?;
                }
                press_chord(&mut input, &modifiers, key)
                    .map_err(|error| flow_like_types::anyhow!("Key chord '{}': {}", text, error))?;
            }
            Ok(())
        })
        .await??;

        session.apply_delay(context).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerHoldKeyNode {}

impl ComputerHoldKeyNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerHoldKeyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_hold_key",
            "Hold Key",
            "Holds a key down for a duration and then releases it. The key is released even when the run fails or is cancelled",
            "Automation/Computer/Keyboard",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "holdKey");
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "key",
            "Key",
            &format!("Key to hold (case-insensitive): {KEY_NAMES}"),
            VariableType::String,
        )
        .set_default_value(Some(json!("shift")));

        node.add_input_pin(
            "duration_ms",
            "Duration (ms)",
            "How long to hold the key (0-60000 ms)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(500)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use enigo::{Direction, Keyboard};

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let key_name: String = context.evaluate_pin("key").await?;
        let duration_ms: i64 = context.evaluate_pin("duration_ms").await?;
        if !(0..=60_000).contains(&duration_ms) {
            return Err(flow_like_types::anyhow!(
                "Hold duration must be between 0 and 60000 ms, got {}",
                duration_ms
            ));
        }
        let key = parse_key(&key_name)?;

        let mut input = session.create_enigo(context).await?;
        let cancellation = context.get_cancellation_token();
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            input
                .key(key, Direction::Press)
                .map_err(|e| flow_like_types::anyhow!("Failed to press key {}: {}", key_name, e))?;
            let held = crate::computer::mouse::interruptible_sleep(
                duration_ms as u64,
                cancellation.as_ref(),
            );
            let released = input.key(key, Direction::Release).map_err(|e| {
                flow_like_types::anyhow!("Failed to release key {}: {}", key_name, e)
            });
            held?;
            released
        })
        .await??;

        session.apply_delay(context).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerTypeSecretNode {}

impl ComputerTypeSecretNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerTypeSecretNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_type_secret",
            "Type Secret",
            "Enters a password or other secret into the focused field without logging it. Type mode sends keystrokes; paste mode puts the secret on the clipboard (excluded from clipboard history where supported), presses the paste shortcut and restores the previous text or image clipboard afterwards",
            "Automation/Computer/Keyboard",
        );
        node.set_version(1);
        node.set_flowscript_name("computer", "typeSecret");
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(7)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "secret",
            "Secret",
            "Secret to enter; connect it from a secret or variable so it is not stored in the board",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());

        node.add_input_pin(
            "mode",
            "Mode",
            "type: send keystrokes. paste: paste through the clipboard, then restore it (use for fields that drop fast keystrokes)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["type".to_string(), "paste".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("type")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use enigo::Keyboard;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let secret: String = context
            .evaluate_pin("secret")
            .await
            .map_err(|_| flow_like_types::anyhow!("Type Secret needs a connected Secret value"))?;
        if secret.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Type Secret received an empty secret"
            ));
        }
        let mode: String = context.evaluate_pin("mode").await?;
        let paste = match mode.as_str() {
            "type" => false,
            "paste" => true,
            other => {
                return Err(flow_like_types::anyhow!(
                    "Unknown Type Secret mode '{}'; use type or paste",
                    other
                ));
            }
        };

        let mut input = session.create_enigo(context).await?;
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            if paste {
                paste_secret(&mut input, &secret)
            } else {
                input
                    .text(&secret)
                    .map_err(|e| flow_like_types::anyhow!("Failed to type the secret: {}", e))
            }
        })
        .await??;

        session.apply_delay(context).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
enum PreviousClipboard {
    Text(String),
    Image(arboard::ImageData<'static>),
    Empty,
}

#[cfg(feature = "execute")]
fn paste_secret(
    input: &mut super::native::input::DesktopInput,
    secret: &str,
) -> flow_like_types::Result<()> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|e| flow_like_types::anyhow!("Clipboard unavailable for secret paste: {}", e))?;
    let previous = match clipboard.get_text() {
        Ok(text) => PreviousClipboard::Text(text),
        Err(_) => clipboard
            .get_image()
            .map(PreviousClipboard::Image)
            .unwrap_or(PreviousClipboard::Empty),
    };
    let set = clipboard.set();
    #[cfg(target_os = "macos")]
    let set = {
        use arboard::SetExtApple;
        set.exclude_from_history()
    };
    #[cfg(target_os = "windows")]
    let set = {
        use arboard::SetExtWindows;
        set.exclude_from_monitoring()
    };
    set.text(secret)
        .map_err(|e| flow_like_types::anyhow!("Failed to place the secret on the clipboard: {}", e))?;
    let primary = if cfg!(target_os = "macos") {
        enigo::Key::Meta
    } else {
        enigo::Key::Control
    };
    let pasted = press_chord(input, &[primary], enigo::Key::Unicode('v'));
    std::thread::sleep(std::time::Duration::from_millis(250));
    let restored = match previous {
        PreviousClipboard::Text(text) => clipboard.set_text(text),
        PreviousClipboard::Image(image) => clipboard.set_image(image),
        PreviousClipboard::Empty => clipboard.clear(),
    };
    pasted.map_err(|e| flow_like_types::anyhow!("Failed to paste the secret: {}", e))?;
    restored.map_err(|e| {
        flow_like_types::anyhow!(
            "The secret was pasted but the previous clipboard could not be restored: {}",
            e
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords_parse_modifiers_in_order_and_normalize_letters() {
        let chord = parse_key_chord("Ctrl + Shift + S").unwrap();
        assert_eq!(
            chord.modifiers,
            vec![ChordModifier::Control, ChordModifier::Shift]
        );
        assert_eq!(chord.key, "s");
        let chord = parse_key_chord("cmd+space").unwrap();
        assert_eq!(chord.modifiers, vec![ChordModifier::Meta]);
        assert_eq!(chord.key, "space");
        assert_eq!(parse_key_chord("alt+F4").unwrap().key, "F4");
    }

    #[test]
    fn chord_aliases_duplicates_and_plus_key() {
        for alias in ["super", "win", "meta", "command"] {
            assert_eq!(
                parse_key_chord(&format!("{alias}+l")).unwrap().modifiers,
                vec![ChordModifier::Meta]
            );
        }
        assert_eq!(
            parse_key_chord("ctrl+ctrl+a").unwrap().modifiers,
            vec![ChordModifier::Control]
        );
        let plus = parse_key_chord("ctrl++").unwrap();
        assert_eq!(
            (plus.modifiers, plus.key.as_str()),
            (vec![ChordModifier::Control], "+")
        );
        assert_eq!(parse_key_chord("+").unwrap().key, "+");
        let primary = parse_key_chord("primary+c").unwrap().modifiers;
        assert_eq!(
            primary,
            vec![if cfg!(target_os = "macos") {
                ChordModifier::Meta
            } else {
                ChordModifier::Control
            }]
        );
        assert_eq!(parse_key_chord("Escape").unwrap().modifiers, vec![]);
    }

    #[test]
    fn malformed_chords_name_the_problem() {
        assert!(parse_key_chord("   ").is_err());
        assert!(parse_key_chord("ctrl+").is_err());
        let error = parse_key_chord("ctrl+banana+s").unwrap_err().to_string();
        assert!(error.contains("banana"), "{error}");
        assert!(parse_key_chord("ctrl++s").is_err());
    }

    #[cfg(feature = "execute")]
    #[test]
    fn key_names_map_to_platform_keys() {
        use enigo::Key;
        assert_eq!(parse_key("Enter").unwrap(), Key::Return);
        assert_eq!(parse_key("pgdn").unwrap(), Key::PageDown);
        assert_eq!(parse_key("F13").unwrap(), Key::F13);
        assert_eq!(parse_key("f20").unwrap(), Key::F20);
        assert_eq!(parse_key("Numpad7").unwrap(), Key::Numpad7);
        assert_eq!(parse_key("NumpadAdd").unwrap(), Key::Add);
        assert_eq!(parse_key("VolumeMute").unwrap(), Key::VolumeMute);
        assert_eq!(parse_key("super").unwrap(), Key::Meta);
        assert_eq!(parse_key("+").unwrap(), Key::Unicode('+'));
        assert_eq!(parse_key(" ").unwrap(), Key::Unicode(' '));
        assert!(parse_key("f25").is_err());
        assert!(parse_key("hyper").unwrap_err().to_string().contains("Unknown key"));
        #[cfg(target_os = "macos")]
        for name in ["Insert", "PrintScreen", "NumLock", "ScrollLock", "Pause", "F21", "F24"] {
            let error = parse_key(name).unwrap_err().to_string();
            assert!(error.contains("cannot be sent"), "{name}: {error}");
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(parse_key("Insert").unwrap(), Key::Insert);
            assert_eq!(parse_key("F24").unwrap(), Key::F24);
        }
    }
}

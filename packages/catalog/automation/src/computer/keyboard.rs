use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

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
            "Key to press (e.g., 'a', 'Enter', 'Tab', 'Escape')",
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
        use enigo::{Direction, Key, Keyboard};

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let key_str: String = context.evaluate_pin("key").await?;
        let modifiers: String = context.evaluate_pin("modifiers").await?;

        {
            let mut enigo = session.create_enigo()?;

            let mods: Vec<&str> = if modifiers.is_empty() {
                Vec::new()
            } else {
                modifiers.split(',').map(|s| s.trim()).collect()
            };

            for modifier in &mods {
                let mod_key = match *modifier {
                    "ctrl" | "control" => Some(Key::Control),
                    "shift" => Some(Key::Shift),
                    "alt" => Some(Key::Alt),
                    "meta" | "cmd" | "command" | "win" => Some(Key::Meta),
                    _ => None,
                };
                if let Some(k) = mod_key {
                    enigo
                        .key(k, Direction::Press)
                        .map_err(|e| flow_like_types::anyhow!("Failed to press modifier: {}", e))?;
                }
            }

            let key = match key_str.to_lowercase().as_str() {
                "enter" | "return" => Key::Return,
                "tab" => Key::Tab,
                "escape" | "esc" => Key::Escape,
                "backspace" => Key::Backspace,
                "delete" => Key::Delete,
                "space" => Key::Space,
                "up" | "arrowup" => Key::UpArrow,
                "down" | "arrowdown" => Key::DownArrow,
                "left" | "arrowleft" => Key::LeftArrow,
                "right" | "arrowright" => Key::RightArrow,
                "home" => Key::Home,
                "end" => Key::End,
                "pageup" => Key::PageUp,
                "pagedown" => Key::PageDown,
                "f1" => Key::F1,
                "f2" => Key::F2,
                "f3" => Key::F3,
                "f4" => Key::F4,
                "f5" => Key::F5,
                "f6" => Key::F6,
                "f7" => Key::F7,
                "f8" => Key::F8,
                "f9" => Key::F9,
                "f10" => Key::F10,
                "f11" => Key::F11,
                "f12" => Key::F12,
                s if s.len() == 1 => Key::Unicode(s.chars().next().unwrap()),
                _ => Key::Unicode(key_str.chars().next().unwrap_or(' ')),
            };

            enigo
                .key(key, Direction::Click)
                .map_err(|e| flow_like_types::anyhow!("Failed to press key: {}", e))?;

            for modifier in mods.iter().rev() {
                let mod_key = match *modifier {
                    "ctrl" | "control" => Some(Key::Control),
                    "shift" => Some(Key::Shift),
                    "alt" => Some(Key::Alt),
                    "meta" | "cmd" | "command" | "win" => Some(Key::Meta),
                    _ => None,
                };
                if let Some(k) = mod_key {
                    enigo.key(k, Direction::Release).map_err(|e| {
                        flow_like_types::anyhow!("Failed to release modifier: {}", e)
                    })?;
                }
            }
        }

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
            "Types text using the keyboard",
            "Automation/Computer/Keyboard",
        );
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
        let text: String = context.evaluate_pin("text").await?;

        {
            let mut enigo = session.create_enigo()?;
            enigo
                .text(&text)
                .map_err(|e| flow_like_types::anyhow!("Failed to type text: {}", e))?;
        }

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

use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{async_trait, json::json};

/// Node to get text from the system clipboard
#[crate::register_node]
#[derive(Default)]
pub struct ClipboardGetTextNode {}

impl ClipboardGetTextNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ClipboardGetTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_clipboard_get_text",
            "Get Clipboard Text",
            "Gets the current text content from the system clipboard",
            "Automation/Computer/Clipboard",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(9)
                .set_governance(4)
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

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "text",
            "Text",
            "Text content from clipboard",
            VariableType::String,
        );

        node.add_output_pin(
            "has_text",
            "Has Text",
            "Whether the clipboard contains text",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use arboard::Clipboard;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;

        let result = std::thread::spawn(|| {
            let mut clipboard = Clipboard::new()?;
            clipboard.get_text()
        })
        .join()
        .map_err(|_| flow_like_types::anyhow!("Clipboard thread panicked"))?;

        match result {
            Ok(text) => {
                context.set_pin_value("text", json!(text)).await?;
                context.set_pin_value("has_text", json!(true)).await?;
            }
            Err(_) => {
                context.set_pin_value("text", json!("")).await?;
                context.set_pin_value("has_text", json!(false)).await?;
            }
        }

        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Clipboard access requires the 'execute' feature"
        ))
    }
}

/// Node to set text to the system clipboard
#[crate::register_node]
#[derive(Default)]
pub struct ClipboardSetTextNode {}

impl ClipboardSetTextNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ClipboardSetTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_clipboard_set_text",
            "Set Clipboard Text",
            "Sets text content to the system clipboard",
            "Automation/Computer/Clipboard",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(9)
                .set_governance(4)
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
            "text",
            "Text",
            "Text to copy to clipboard",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "exec_error",
            "Error",
            "Failed to set clipboard",
            VariableType::Execution,
        );

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
        use arboard::Clipboard;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let text: String = context.evaluate_pin("text").await?;

        let result = std::thread::spawn(move || {
            let mut clipboard = Clipboard::new()?;
            clipboard.set_text(&text)
        })
        .join()
        .map_err(|_| flow_like_types::anyhow!("Clipboard thread panicked"))?;

        context.set_pin_value("session_out", json!(session)).await?;

        match result {
            Ok(()) => {
                context.activate_exec_pin("exec_out").await?;
            }
            Err(_) => {
                context.activate_exec_pin("exec_error").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Clipboard access requires the 'execute' feature"
        ))
    }
}

/// Node to get image from the system clipboard
#[crate::register_node]
#[derive(Default)]
pub struct ClipboardGetImageNode {}

impl ClipboardGetImageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ClipboardGetImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_clipboard_get_image",
            "Get Clipboard Image",
            "Gets an image from the system clipboard if available",
            "Automation/Computer/Clipboard",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(7)
                .set_governance(4)
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

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "exec_no_image",
            "No Image",
            "Clipboard does not contain an image",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "image",
            "Image",
            "Image from clipboard as NodeImage",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node.add_output_pin(
            "has_image",
            "Has Image",
            "Whether the clipboard contains an image",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use arboard::Clipboard;
        use flow_like_types::image::{DynamicImage, RgbaImage};

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_no_image").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;

        let result = std::thread::spawn(|| {
            let mut clipboard = Clipboard::new()?;
            clipboard.get_image()
        })
        .join()
        .map_err(|_| flow_like_types::anyhow!("Clipboard thread panicked"))?;

        context.set_pin_value("session_out", json!(session)).await?;

        match result {
            Ok(img_data) => {
                let rgba_image = RgbaImage::from_raw(
                    img_data.width as u32,
                    img_data.height as u32,
                    img_data.bytes.into_owned(),
                )
                .ok_or_else(|| {
                    flow_like_types::anyhow!("Failed to create image from clipboard data")
                })?;

                let dyn_image = DynamicImage::ImageRgba8(rgba_image);
                let node_image = NodeImage::new(context, dyn_image).await;

                context.set_pin_value("image", json!(node_image)).await?;
                context.set_pin_value("has_image", json!(true)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(_) => {
                context.set_pin_value("image", json!(null)).await?;
                context.set_pin_value("has_image", json!(false)).await?;
                context.activate_exec_pin("exec_no_image").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Clipboard access requires the 'execute' feature"
        ))
    }
}

/// Node to set image to the system clipboard
#[crate::register_node]
#[derive(Default)]
pub struct ClipboardSetImageNode {}

impl ClipboardSetImageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ClipboardSetImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_clipboard_set_image",
            "Set Clipboard Image",
            "Sets an image to the system clipboard",
            "Automation/Computer/Clipboard",
        );
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(7)
                .set_governance(4)
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
            "image",
            "Image",
            "Image to copy to clipboard (NodeImage)",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "exec_error",
            "Error",
            "Failed to set clipboard image",
            VariableType::Execution,
        );

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
        use arboard::{Clipboard, ImageData};
        use std::borrow::Cow;

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_error").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let node_image: NodeImage = context.evaluate_pin("image").await?;

        // Get the actual image from the cache using NodeImage's get_image method
        let image_arc = node_image.get_image(context).await?;
        let guard = image_arc.lock().await;
        let rgba = guard.to_rgba8();
        let width = rgba.width() as usize;
        let height = rgba.height() as usize;
        let bytes = rgba.into_raw();
        drop(guard);

        let result = std::thread::spawn(move || {
            let mut clipboard = Clipboard::new()?;
            let img_data = ImageData {
                width,
                height,
                bytes: Cow::Owned(bytes),
            };
            clipboard.set_image(img_data)
        })
        .join()
        .map_err(|_| flow_like_types::anyhow!("Clipboard thread panicked"))?;

        context.set_pin_value("session_out", json!(session)).await?;

        match result {
            Ok(()) => {
                context.activate_exec_pin("exec_out").await?;
            }
            Err(_) => {
                context.activate_exec_pin("exec_error").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Clipboard access requires the 'execute' feature"
        ))
    }
}

use flow_like::flow::{
    execution::{context::ExecutionContext, device::local_device_command},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{
    Value, async_trait,
    base64::{Engine, engine::general_purpose::STANDARD},
    json::json,
};
use std::time::Duration;

fn clipboard_node(image: bool) -> Node {
    let mut node = Node::new(
        if image {
            "computer_clipboard_set_image"
        } else {
            "computer_clipboard_set_text"
        },
        if image {
            "Set Clipboard Image"
        } else {
            "Set Clipboard Text"
        },
        "Writes to the local device clipboard, or to the calling frontend when this Event runs remotely",
        "Automation/Computer/Clipboard",
    );
    node.set_flowscript_name(
        "computer",
        if image {
            "clipboardSetImage"
        } else {
            "clipboardSetText"
        },
    );
    node.add_icon("/flow/icons/computer.svg");
    node.set_version(1);
    node.set_scores(
        NodeScores::new()
            .set_privacy(2)
            .set_security(3)
            .set_performance(if image { 7 } else { 9 })
            .set_governance(4)
            .set_reliability(if image { 7 } else { 8 })
            .set_cost(10)
            .build(),
    );
    node.set_long_running(true);
    node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);
    node.add_input_pin(
        "session",
        "Session",
        "Optional automation session, passed through for existing flows",
        VariableType::Struct,
    )
    .set_options(PinOptions::new().set_optional(true).build())
    .set_default_value(Some(Value::Null));
    if image {
        node.add_input_pin("image", "Image", "Image to copy", VariableType::Struct)
            .set_schema::<NodeImage>();
    } else {
        node.add_input_pin("text", "Text", "Text to copy", VariableType::String);
        node.add_input_pin(
            "html",
            "HTML",
            "Optional rich text, with Text as the plain-text fallback",
            VariableType::String,
        )
        .set_default_value(Some(json!("")))
        .set_options(PinOptions::new().set_optional(true).build());
    }
    node.add_input_pin(
        "local_only",
        "Keep on device",
        "Prevent cross-device clipboard sharing on supported platforms",
        VariableType::Boolean,
    )
    .set_default_value(Some(json!(false)));
    node.add_input_pin("expires_in_seconds", "Expire after", "Seconds before content expires; zero leaves it on the clipboard. Requires platform support", VariableType::Integer)
        .set_default_value(Some(json!(0)));
    node.add_input_pin(
        "timeout_seconds",
        "Client timeout",
        "Maximum time to wait for the invoking client",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(30)));
    node.add_output_pin(
        "exec_out",
        "▶",
        "Clipboard write acknowledged",
        VariableType::Execution,
    );
    node.add_output_pin(
        "exec_error",
        "Error",
        "Clipboard write failed or was not acknowledged",
        VariableType::Execution,
    );
    node.add_output_pin(
        "session_out",
        "Session",
        "Optional automation session",
        VariableType::Struct,
    );
    node.add_output_pin(
        "error",
        "Error",
        "Structured error code and message",
        VariableType::Struct,
    );
    node
}

async fn write(context: &mut ExecutionContext, image: bool) -> flow_like_types::Result<()> {
    context.deactivate_exec_pin("exec_out").await?;
    context.deactivate_exec_pin("exec_error").await?;
    let session = context
        .evaluate_pin::<Value>("session")
        .await
        .unwrap_or(Value::Null);
    context.set_pin_value("session_out", session).await?;
    let local_only = context
        .evaluate_pin::<bool>("local_only")
        .await
        .unwrap_or(false);
    let expires = context
        .evaluate_pin::<i64>("expires_in_seconds")
        .await
        .unwrap_or(0);
    let timeout = context
        .evaluate_pin::<i64>("timeout_seconds")
        .await
        .unwrap_or(30)
        .clamp(1, 120);
    let mut args = if image {
        let node_image: NodeImage = context.evaluate_pin("image").await?;
        let image = node_image.get_image(context).await?;
        let image = image.lock().await;
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, flow_like_types::image::ImageFormat::Png)?;
        if bytes.get_ref().len() > 16 * 1024 * 1024 {
            return finish(context, json!({"ok":false,"error":{"code":"payload_too_large","message":"Clipboard images must be at most 16 MiB encoded"}})).await;
        }
        json!({"format":"image","imageBase64":STANDARD.encode(bytes.into_inner())})
    } else {
        let text: String = context.evaluate_pin("text").await?;
        if text.len() > 1024 * 1024 {
            return finish(context, json!({"ok":false,"error":{"code":"payload_too_large","message":"Clipboard text must be at most 1 MiB"}})).await;
        }
        let html = context
            .evaluate_pin::<String>("html")
            .await
            .unwrap_or_default();
        if html.len() > 1024 * 1024 {
            return finish(context, json!({"ok":false,"error":{"code":"payload_too_large","message":"Clipboard HTML must be at most 1 MiB"}})).await;
        }
        if html.is_empty() {
            json!({"format":"text","text":text})
        } else {
            json!({"format":"html","text":text,"html":html})
        }
    };
    args["localOnly"] = json!(local_only);
    if expires > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;
        args["expiresAt"] = json!(now.saturating_add(expires.min(86400) as u64 * 1000));
    }
    let result = if context.execution_environment().is_local() {
        let cancellation = context.cancellation_token().unwrap_or_default();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;
        args["requestDeadline"] = json!(now.saturating_add(timeout as u64 * 1000));
        let operation = async {
            match local_device_command("clipboard.write", args.clone()).await {
                Some(result) => result,
                None => write_local(args, cancellation.clone()).await,
            }
        };
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Ok(json!({"ok":false,"error":{"code":"cancelled","message":"Clipboard request was cancelled"}})),
            result = tokio::time::timeout(Duration::from_secs(timeout as u64), operation) => result.unwrap_or_else(|_| Ok(json!({"ok":false,"error":{"code":"device_timeout","message":"Clipboard device did not respond before its deadline"}}))),
        }
    } else {
        context
            .request_device("clipboard.write", args, Duration::from_secs(timeout as u64))
            .await
    };
    let response = result.unwrap_or_else(
        |error| json!({"ok":false,"error":{"code":"clipboard_failed","message":error.to_string()}}),
    );
    finish(context, response).await
}

async fn finish(context: &mut ExecutionContext, response: Value) -> flow_like_types::Result<()> {
    let ok = response.get("ok").and_then(Value::as_bool) == Some(true);
    // Older compiled flows do not carry the structured error output yet.
    if let Ok(pin) = context.get_pin_by_name("error").await {
        context
            .set_pin_ref_value(&pin, response.get("error").cloned().unwrap_or(Value::Null))
            .await?;
    }
    context
        .activate_exec_pin(if ok { "exec_out" } else { "exec_error" })
        .await?;
    Ok(())
}

async fn write_local(
    args: Value,
    cancellation: flow_like_types::tokio_util::sync::CancellationToken,
) -> flow_like_types::Result<Value> {
    #[cfg(all(
        feature = "local-clipboard",
        not(any(target_os = "ios", target_os = "android"))
    ))]
    {
        return tokio::task::spawn_blocking(move || -> flow_like_types::Result<Value> {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64;
            if cancellation.is_cancelled() || args["requestDeadline"].as_u64().is_some_and(|deadline| deadline <= now) {
                return Ok(json!({"ok":false,"error":{"code":"cancelled","message":"Clipboard request ended before writing"}}));
            }
            if args["localOnly"] == true || args.get("expiresAt").is_some() {
                return Ok(json!({"ok":false,"error":{"code":"unsupported_options","message":"This clipboard host does not support local-only content or expiration"}}));
            }
            let mut clipboard = arboard::Clipboard::new()?;
            if args["format"] == "image" {
                let bytes = STANDARD.decode(args["imageBase64"].as_str().unwrap_or_default())?;
                let rgba = flow_like_types::image::load_from_memory(&bytes)?.to_rgba8();
                clipboard.set_image(arboard::ImageData {width:rgba.width() as usize,height:rgba.height() as usize,bytes:std::borrow::Cow::Owned(rgba.into_raw())})?;
            } else if args["format"] == "html" {
                clipboard.set_html(args["html"].as_str().unwrap_or_default(), args["text"].as_str())?;
            } else {
                clipboard.set_text(args["text"].as_str().unwrap_or_default())?;
            }
            Ok(json!({"ok":true,"value":{"format":args["format"]}}))
        }).await?;
    }
    #[cfg(not(all(
        feature = "local-clipboard",
        not(any(target_os = "ios", target_os = "android"))
    )))]
    {
        let _ = (args, cancellation);
        Ok(
            json!({"ok":false,"error":{"code":"device_unavailable","message":"No local clipboard provider is installed"}}),
        )
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ClipboardSetTextNode;
impl ClipboardSetTextNode {
    pub fn new() -> Self {
        Self
    }
}
#[async_trait]
impl NodeLogic for ClipboardSetTextNode {
    fn get_node(&self) -> Node {
        clipboard_node(false)
    }
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        write(context, false).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ClipboardSetImageNode;
impl ClipboardSetImageNode {
    pub fn new() -> Self {
        Self
    }
}
#[async_trait]
impl NodeLogic for ClipboardSetImageNode {
    fn get_node(&self) -> Node {
        clipboard_node(true)
    }
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        write(context, true).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::board::{Board, cleanup::sync_node_schema::sync_board_node_schemas};
    use flow_like::state::FlowNodeRegistryInner;
    use std::sync::Arc;

    #[tokio::test]
    async fn old_placed_writers_gain_device_pins_without_losing_session_connections() {
        let writers: Vec<Arc<dyn NodeLogic>> = vec![
            Arc::new(ClipboardSetTextNode),
            Arc::new(ClipboardSetImageNode),
        ];
        for writer in writers {
            let catalog = writer.get_node();
            let mut placed = catalog.clone();
            placed.version = None;
            placed.only_offline = true;
            placed.pins.retain(|_, pin| {
                matches!(
                    pin.name.as_str(),
                    "exec_in"
                        | "session"
                        | "text"
                        | "image"
                        | "exec_out"
                        | "exec_error"
                        | "session_out"
                )
            });
            let session_schema = r#"{"type":"object","title":"AutomationSession","properties":{"session_ref":{"type":"string"}}}"#;
            for pin in placed
                .pins
                .values_mut()
                .filter(|pin| matches!(pin.name.as_str(), "session" | "session_out"))
            {
                pin.schema = Some(session_schema.into());
                pin.options = None;
                pin.default_value = None;
            }
            let input = placed.get_pin_by_name("session").unwrap().id.clone();
            let output = placed.get_pin_by_name("session_out").unwrap().id.clone();
            let mut previous = Node::new("previous-session", "Previous", "", "");
            let peer = previous.add_output_pin("session", "Session", "", VariableType::Struct);
            peer.schema = Some(session_schema.into());
            peer.connected_to.insert(input.clone());
            let previous_pin = peer.id.clone();
            placed
                .pins
                .get_mut(&input)
                .unwrap()
                .depends_on
                .insert(previous_pin.clone());
            let mut next = Node::new("next-session", "Next", "", "");
            let peer = next.add_input_pin("session", "Session", "", VariableType::Struct);
            peer.schema = Some(session_schema.into());
            peer.depends_on.insert(output.clone());
            let next_pin = peer.id.clone();
            placed
                .pins
                .get_mut(&output)
                .unwrap()
                .connected_to
                .insert(next_pin.clone());
            let node_id = placed.id.clone();
            let mut board = Board::new_detached(None, flow_like_storage::Path::default());
            for node in [previous, placed, next] {
                board.nodes.insert(node.id.clone(), node);
            }
            let mut registry = FlowNodeRegistryInner::new(1);
            registry.insert(catalog, writer);

            sync_board_node_schemas(&mut board, &registry).await;
            board.cleanup();

            let migrated = &board.nodes[&node_id];
            assert_eq!(migrated.version, Some(1));
            assert!(!migrated.only_offline);
            assert!(migrated.get_pin_by_name("error").is_some());
            assert!(migrated.get_pin_by_name("timeout_seconds").is_some());
            assert!(migrated.get_pin_by_name("local_only").is_some());
            assert!(migrated.get_pin_by_name("session").unwrap().is_optional());
            assert_eq!(migrated.get_pin_by_name("session").unwrap().id, input);
            assert_eq!(migrated.get_pin_by_name("session_out").unwrap().id, output);
            assert!(migrated.pins[&input].depends_on.contains(&previous_pin));
            assert!(migrated.pins[&output].connected_to.contains(&next_pin));
            assert!(flow_like::flow::pin::schemas_are_compatible(
                migrated.pins[&output].schema.as_deref(),
                Some(session_schema)
            ));
        }
    }

    #[test]
    fn writes_remain_compatible_and_allow_remote_execution() {
        for (node, name) in [
            (
                ClipboardSetTextNode.get_node(),
                "computer_clipboard_set_text",
            ),
            (
                ClipboardSetImageNode.get_node(),
                "computer_clipboard_set_image",
            ),
        ] {
            assert_eq!(node.name, name);
            assert_eq!(node.version, Some(1));
            assert!(!node.only_offline);
            assert!(node.get_pin_by_name("session").unwrap().is_optional());
            assert!(node.get_pin_by_name("exec_error").is_some());
            assert!(node.get_pin_by_name("session_out").is_some());
        }
    }
}

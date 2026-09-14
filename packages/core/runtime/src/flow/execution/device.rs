use super::context::ExecutionContext;
use flow_like_types::channel::ChannelOutcome;
use flow_like_types::{Value, anyhow, json::json};
use futures::future::BoxFuture;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

pub type LocalDeviceHandler =
    Arc<dyn Fn(String, Value) -> BoxFuture<'static, flow_like_types::Result<Value>> + Send + Sync>;
static LOCAL_DEVICE_HANDLER: OnceLock<RwLock<Option<LocalDeviceHandler>>> = OnceLock::new();

/// Installed by a native host. Server execution never calls this handler.
pub fn set_local_device_handler(handler: LocalDeviceHandler) {
    *LOCAL_DEVICE_HANDLER
        .get_or_init(|| RwLock::new(None))
        .write()
        .unwrap_or_else(|e| e.into_inner()) = Some(handler);
}

pub async fn local_device_command(
    command: &str,
    args: Value,
) -> Option<flow_like_types::Result<Value>> {
    let handler = LOCAL_DEVICE_HANDLER
        .get_or_init(|| RwLock::new(None))
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()?;
    Some(handler(command.to_owned(), args).await)
}

impl ExecutionContext {
    /// Ask the invoking client to perform a device operation. The channel is
    /// scoped to this run, including when the Event executes on a remote worker.
    pub async fn request_device(
        &mut self,
        command: &str,
        args: Value,
        timeout: Duration,
    ) -> flow_like_types::Result<Value> {
        if self.is_cancelled() {
            return Ok(
                json!({"ok":false,"error":{"code":"cancelled","message":"The device request was cancelled"}}),
            );
        }
        let max_timeout = if command == "audio.play" { 600 } else { 120 };
        let timeout = timeout.clamp(Duration::from_millis(100), Duration::from_secs(max_timeout));
        let Some(cache) = &self.execution_cache else {
            return Ok(
                json!({"ok":false,"error":{"code":"frontend_unavailable","message":"No invoking client is attached to this execution"}}),
            );
        };
        let app_id = cache.app_id.clone();
        let channel = match self.channel() {
            Ok(channel) => channel,
            Err(_) => {
                return Ok(
                    json!({"ok":false,"error":{"code":"frontend_unavailable","message":"No client reply channel is attached to this execution"}}),
                );
            }
        };
        let ticket = channel.open(timeout).await?;
        let message = crate::a2ui::A2UIServerMessage::DeviceCommand {
            request_id: ticket.request_id.clone(),
            command: command.to_owned(),
            args,
            timeout_ms: timeout.as_millis() as u64,
            app_id,
            channel: ticket.handle.clone(),
        };
        if let Err(error) = self.stream_a2ui_update(message).await {
            channel.abandon(&ticket).await;
            return Err(error);
        }
        let result = match channel.wait(&ticket, self.cancellation_token()).await? {
            ChannelOutcome::Responded(value) => value,
            ChannelOutcome::Expired => {
                json!({"ok":false,"error":{"code":"frontend_timeout","message":"The invoking client did not acknowledge the device operation before its deadline"}})
            }
            ChannelOutcome::Cancelled => {
                json!({"ok":false,"error":{"code":"cancelled","message":"The device request was cancelled"}})
            }
            ChannelOutcome::Closed => {
                json!({"ok":false,"error":{"code":"frontend_unavailable","message":"The client reply channel closed"}})
            }
        };
        if !result.get("ok").is_some_and(Value::is_boolean) {
            return Err(anyhow!("Invalid device response: missing acknowledgement"));
        }
        Ok(result)
    }
}

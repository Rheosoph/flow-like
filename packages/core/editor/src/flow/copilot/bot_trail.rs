//! A bot turn's tool trail: the calls of the turn, recorded for the host to keep on the reply, and
//! the calls of the bot's earlier replies, shown in the prompt (see
//! [`flow_like_types::bot_tool_trail`]). Hosts supply it in [`super::HostTools::with_trail`].

use std::fmt;
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use flow_like_bot_memory::tools::tool_definitions;
use flow_like_bot_tools as tools;
use flow_like_types::bot_tool_trail::{ToolCall, TrailSection, attach_tool_parts};
use serde_json::Value;

use super::bot_notes::{NOTES_CLOSE_TOOL, NOTES_READ_TOOL, NOTES_WRITE_TOOL};
use super::platform::PlatformToolBridge;
use super::tool_spec::{MEMORY_SEARCH_TOOL, MEMORY_STORE_TOOL};

/// Calls a reply never keeps: notes and memory hold their results in their own stores, user
/// context is shared for one answer or session, and secrets stay out of the conversation.
const UNKEPT_TOOLS: [&str; 10] = [
    NOTES_WRITE_TOOL,
    NOTES_READ_TOOL,
    NOTES_CLOSE_TOOL,
    MEMORY_STORE_TOOL,
    MEMORY_SEARCH_TOOL,
    tools::user_context::USER_CONTEXT_AVAILABLE,
    tools::user_context::USER_CONTEXT_REQUEST,
    tools::user_context::USER_CONTEXT_SET_STATUS,
    tools::secrets::SECRET_LIST,
    tools::secrets::SECRET_REQUEST,
];

static MEMORY_TOOLS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    tool_definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect()
});

fn kept(tool_name: &str) -> bool {
    !UNKEPT_TOOLS.contains(&tool_name) && !MEMORY_TOOLS.contains(&tool_name)
}

struct TrailInner {
    prior: Option<TrailSection>,
    calls: Mutex<Vec<Option<ToolCall>>>,
}

/// The trail of one turn: the section of earlier calls it shows and the calls it makes.
#[derive(Clone)]
pub struct ToolTrail(Arc<TrailInner>);

impl ToolTrail {
    /// `prior` is the section the host built from the bot's earlier replies.
    pub fn new(prior: Option<TrailSection>) -> Self {
        Self(Arc::new(TrailInner {
            prior,
            calls: Mutex::new(Vec::new()),
        }))
    }

    pub fn section(&self) -> Option<&TrailSection> {
        self.0.prior.as_ref()
    }

    /// The finished calls of this turn in the order they started.
    pub fn calls(&self) -> Vec<ToolCall> {
        self.lock().iter().flatten().cloned().collect()
    }

    /// Puts this turn's calls on its reply `body` as tool parts.
    pub fn attach(&self, body: &mut Value) {
        attach_tool_parts(body, &self.calls());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Option<ToolCall>>> {
        self.0
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn begin(&self) -> usize {
        let mut calls = self.lock();
        calls.push(None);
        calls.len() - 1
    }

    fn finish(&self, slot: usize, call: ToolCall) {
        if let Some(entry) = self.lock().get_mut(slot) {
            *entry = Some(call);
        }
    }
}

impl fmt::Debug for ToolTrail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolTrail")
            .field("prior", &self.0.prior.is_some())
            .field("calls", &self.lock().len())
            .finish()
    }
}

impl PartialEq for ToolTrail {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ToolTrail {}

/// Records every call the model makes, with the output it reads, into the turn's trail.
pub(super) struct TrailBridge {
    trail: ToolTrail,
    inner: Arc<dyn PlatformToolBridge>,
}

impl TrailBridge {
    pub(super) fn new(trail: ToolTrail, inner: Arc<dyn PlatformToolBridge>) -> Self {
        Self { trail, inner }
    }
}

#[async_trait]
impl PlatformToolBridge for TrailBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        if !kept(tool_name) {
            return self.inner.call(tool_name, arguments).await;
        }
        let slot = self.trail.begin();
        let output = self.inner.call(tool_name, arguments.clone()).await;
        self.trail.finish(
            slot,
            ToolCall {
                name: tool_name.to_string(),
                arguments,
                output: output.clone(),
            },
        );
        output
    }

    fn supports_tool(&self, tool_name: &str) -> bool {
        self.inner.supports_tool(tool_name)
    }

    async fn drain_steering(&self) -> Vec<String> {
        self.inner.drain_steering().await
    }

    fn watches_room(&self) -> bool {
        self.inner.watches_room()
    }

    async fn drain_room_updates(&self) -> Vec<String> {
        self.inner.drain_room_updates().await
    }

    async fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::bot_message::MessageBody;
    use serde_json::json;

    struct Echo;

    #[async_trait]
    impl PlatformToolBridge for Echo {
        async fn call(&self, tool_name: &str, arguments: Value) -> String {
            json!({ "status": "ok", "tool": tool_name, "args": arguments }).to_string()
        }
    }

    #[tokio::test]
    async fn the_trail_keeps_calls_in_order_and_skips_working_state_tools() {
        let trail = ToolTrail::new(None);
        let bridge = TrailBridge::new(trail.clone(), Arc::new(Echo));
        bridge
            .call("database_tool", json!({ "table_name": "cards" }))
            .await;
        bridge.call(NOTES_WRITE_TOOL, json!({ "key": "k" })).await;
        bridge.call("memory_search", json!({ "query": "x" })).await;
        bridge
            .call(tools::user_context::USER_CONTEXT_REQUEST, json!({}))
            .await;
        bridge
            .call("web_fetch", json!({ "url": "https://x" }))
            .await;

        let names: Vec<String> = trail.calls().into_iter().map(|call| call.name).collect();
        assert_eq!(names, ["database_tool", "web_fetch"]);

        let mut body = json!({ "text": "Karte 3" });
        trail.attach(&mut body);
        let parsed = MessageBody::parse(&body).unwrap();
        assert_eq!(parsed.parts.len(), 2);
        assert_eq!(parsed.parts[1].id(), "t2");
    }
}

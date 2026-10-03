//! Discord session types and management

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(feature = "execute")]
use {
    flow_like_types::{Cacheable, json::json},
    serenity::all::{ChannelId, GuildId, Http, MessageId, UserId},
    std::{
        any::Any,
        collections::HashMap,
        sync::{Arc, OnceLock, PoisonError, RwLock},
    },
};

#[cfg(feature = "execute")]
fn bot_credentials() -> &'static RwLock<HashMap<String, String>> {
    static INSTANCE: OnceLock<RwLock<HashMap<String, String>>> = OnceLock::new();
    INSTANCE.get_or_init(Default::default)
}

/// Lets `handle` stand for `token` inside this process, so a run's payload carries the handle
/// and never the token.
#[cfg(feature = "execute")]
pub fn register_bot_credential(handle: &str, token: &str) {
    bot_credentials()
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(handle.to_string(), token.to_string());
}

#[cfg(feature = "execute")]
pub fn forget_bot_credential(handle: &str) {
    bot_credentials()
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(handle);
}

/// The token a registered handle stands for; any other value is returned unchanged.
#[cfg(feature = "execute")]
pub fn resolve(value: &str) -> String {
    bot_credentials()
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(value)
        .cloned()
        .unwrap_or_else(|| value.to_string())
}

/// Discord user information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct DiscordUser {
    pub id: String,
    pub name: String,
    pub discriminator: Option<u16>,
    pub bot: bool,
}

/// Discord session data stored in global_session
/// Contains all information needed to reconstruct a Discord HTTP client
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscordSessionData {
    pub bot_token: String,
    pub guild_id: Option<String>,
    pub channel_id: String,
    pub message_id: String,
    pub bot_user_id: Option<String>,
    #[serde(default)]
    pub user: Option<DiscordUser>,
}

/// A typed Discord session with a reference to the cached client
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscordSession {
    pub ref_id: String,
    pub guild_id: Option<String>,
    pub channel_id: String,
    pub message_id: String,
    pub bot_user_id: Option<String>,
    #[serde(default)]
    pub user: Option<DiscordUser>,
}

#[cfg(feature = "execute")]
impl DiscordSession {
    pub fn channel_id(&self) -> flow_like_types::Result<ChannelId> {
        Ok(ChannelId::new(self.channel_id.parse()?))
    }

    pub fn message_id(&self) -> flow_like_types::Result<MessageId> {
        Ok(MessageId::new(self.message_id.parse()?))
    }

    pub fn guild_id(&self) -> flow_like_types::Result<Option<GuildId>> {
        match &self.guild_id {
            Some(id) => Ok(Some(GuildId::new(id.parse()?))),
            None => Ok(None),
        }
    }
}

/// Cached Discord HTTP client for making API calls
#[cfg(feature = "execute")]
pub struct CachedDiscordClient {
    pub http: Arc<Http>,
    pub bot_user_id: Option<UserId>,
}

#[cfg(feature = "execute")]
impl Cacheable for CachedDiscordClient {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(feature = "execute")]
impl CachedDiscordClient {
    pub fn new(token: &str) -> Self {
        let http = Arc::new(Http::new(&resolve(token)));
        Self {
            http,
            bot_user_id: None,
        }
    }

    pub async fn with_bot_info(token: &str) -> flow_like_types::Result<Self> {
        let http = Arc::new(Http::new(&resolve(token)));
        let bot_info = http.get_current_user().await?;
        Ok(Self {
            http,
            bot_user_id: Some(bot_info.id),
        })
    }
}

/// Helper to get the cached Discord client from context
#[cfg(feature = "execute")]
pub async fn get_discord_client(
    context: &ExecutionContext,
    ref_id: &str,
) -> flow_like_types::Result<Arc<CachedDiscordClient>> {
    let cache = context.cache.read().await;
    let client = cache
        .get(ref_id)
        .ok_or_else(|| flow_like_types::anyhow!("Discord client not found in cache: {}", ref_id))?
        .clone();

    let client = client
        .as_any()
        .downcast_ref::<CachedDiscordClient>()
        .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast Discord client"))?;

    Ok(Arc::new(CachedDiscordClient {
        http: client.http.clone(),
        bot_user_id: client.bot_user_id,
    }))
}

// ============================================================================
// To Discord Session Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ToDiscordSessionNode;

impl ToDiscordSessionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ToDiscordSessionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_to_session",
            "To Discord Session",
            "Creates a Discord session from local_session data for use with other Discord nodes",
            "Discord",
        );
        node.add_icon("/flow/icons/discord.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "local_session",
            "Local Session",
            "The local_session from a Chat Event containing Discord session data",
            VariableType::Struct,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after session is created",
            VariableType::Execution,
        );

        node.add_output_pin(
            "session",
            "Session",
            "Discord session for use with other Discord nodes",
            VariableType::Struct,
        )
        .set_schema::<DiscordSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session_data: DiscordSessionData = context.evaluate_pin("local_session").await?;

        let ref_id = format!("discord_client_{}", flow_like_types::create_id());

        let client = CachedDiscordClient::with_bot_info(&session_data.bot_token)
            .await
            .unwrap_or_else(|_| CachedDiscordClient::new(&session_data.bot_token));

        let cacheable: Arc<dyn Cacheable> = Arc::new(client);
        context.set_cache(&ref_id, cacheable).await;

        let session = DiscordSession {
            ref_id,
            guild_id: session_data.guild_id,
            channel_id: session_data.channel_id,
            message_id: session_data.message_id,
            bot_user_id: session_data.bot_user_id,
            user: session_data.user,
        };

        context.set_pin_value("session", json!(session)).await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Discord functionality requires the 'execute' feature"
        ))
    }
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;

    #[test]
    fn session_handle_resolves_until_forgotten() {
        let handle = "device-bot:evt_session_resolve";
        register_bot_credential(handle, "first.token.value");
        assert_eq!(resolve(handle), "first.token.value");

        register_bot_credential(handle, "second.token.value");
        assert_eq!(resolve(handle), "second.token.value");

        forget_bot_credential(handle);
        assert_eq!(resolve(handle), handle);
    }

    #[test]
    fn session_values_that_are_no_handle_stay_unchanged() {
        for value in [
            "",
            "MTA4.abc_def.ghi-jkl",
            "device-bot:evt_never_registered",
        ] {
            assert_eq!(resolve(value), value);
        }
    }

    #[test]
    fn session_client_is_built_with_the_token_of_a_handle() {
        let handle = "device-bot:evt_session_client";
        register_bot_credential(handle, "client.token.value");
        assert_eq!(
            CachedDiscordClient::new(handle).http.token(),
            "Bot client.token.value"
        );

        forget_bot_credential(handle);
        assert_eq!(
            CachedDiscordClient::new("plain.token.value").http.token(),
            "Bot plain.token.value"
        );
    }
}

//! Discord user types and conversion

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use serenity::all::UserId;

/// A typed Discord user
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscordUser {
    pub id: String,
    pub name: String,
    pub discriminator: Option<String>,
    pub is_bot: bool,
    pub avatar_url: Option<String>,
}

#[cfg(feature = "execute")]
impl DiscordUser {
    pub fn user_id(&self) -> flow_like_types::Result<UserId> {
        Ok(UserId::new(self.id.parse()?))
    }
}

/// Generic user struct from chat events
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GenericUser {
    pub sub: String,
    pub name: String,
    pub bot: Option<bool>,
}

// ============================================================================
// To Discord User Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct ToDiscordUserNode;

impl ToDiscordUserNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ToDiscordUserNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "discord_to_user",
            "To Discord User",
            "Converts a generic user (from Chat Event) to a typed Discord user",
            "Discord",
        );
        node.add_icon("/flow/icons/discord.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "user",
            "User",
            "Generic user from Chat Event",
            VariableType::Struct,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after conversion",
            VariableType::Execution,
        );

        node.add_output_pin(
            "discord_user",
            "Discord User",
            "Typed Discord user",
            VariableType::Struct,
        )
        .set_schema::<DiscordUser>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let user: GenericUser = context.evaluate_pin("user").await?;

        let discord_user = DiscordUser {
            id: user.sub,
            name: user.name,
            discriminator: None,
            is_bot: user.bot.unwrap_or(false),
            avatar_url: None,
        };

        context
            .set_pin_value("discord_user", json!(discord_user))
            .await?;

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

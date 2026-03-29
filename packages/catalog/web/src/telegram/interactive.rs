//! Telegram interactive elements (polls, dice, locations, contacts)

use super::message::SentMessage;
use super::session::TelegramSession;
#[cfg(feature = "execute")]
use super::session::get_telegram_bot;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use teloxide::prelude::*;
#[cfg(feature = "execute")]
use teloxide::types::InputPollOption;

/// Dice emoji types
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum DiceEmoji {
    Dice,
    Darts,
    Basketball,
    Football,
    Bowling,
    SlotMachine,
}

impl DiceEmoji {
    fn to_teloxide(&self) -> teloxide::types::DiceEmoji {
        match self {
            DiceEmoji::Dice => teloxide::types::DiceEmoji::Dice,
            DiceEmoji::Darts => teloxide::types::DiceEmoji::Darts,
            DiceEmoji::Basketball => teloxide::types::DiceEmoji::Basketball,
            DiceEmoji::Football => teloxide::types::DiceEmoji::Football,
            DiceEmoji::Bowling => teloxide::types::DiceEmoji::Bowling,
            DiceEmoji::SlotMachine => teloxide::types::DiceEmoji::SlotMachine,
        }
    }
}

// ============================================================================
// Send Dice Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendDiceNode;

impl SendDiceNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendDiceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_dice",
            "Send Dice",
            "Sends an animated dice/game emoji with random result",
            "Telegram/Interactive",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "emoji",
            "Emoji Type",
            "Type of dice animation (Dice, Darts, Basketball, Football, Bowling, SlotMachine)",
            VariableType::String,
        )
        .set_default_value(Some(json!("Dice")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Dice".into(),
                    "Darts".into(),
                    "Basketball".into(),
                    "Football".into(),
                    "Bowling".into(),
                    "SlotMachine".into(),
                ])
                .build(),
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after dice is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "value",
            "Result Value",
            "The random result value of the dice",
            VariableType::Integer,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let emoji_str: String = context
            .evaluate_pin::<String>("emoji")
            .await
            .unwrap_or_else(|_| "Dice".to_string());

        let emoji = match emoji_str.as_str() {
            "Darts" => DiceEmoji::Darts,
            "Basketball" => DiceEmoji::Basketball,
            "Football" => DiceEmoji::Football,
            "Bowling" => DiceEmoji::Bowling,
            "SlotMachine" => DiceEmoji::SlotMachine,
            _ => DiceEmoji::Dice,
        };

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let sent = bot
            .bot
            .send_dice(chat_id)
            .emoji(emoji.to_teloxide())
            .await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        let dice_value = sent.dice().map(|d| d.value as i64).unwrap_or(0);

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;
        context.set_pin_value("value", json!(dice_value)).await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Send Poll Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendPollNode;

impl SendPollNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendPollNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_poll",
            "Send Poll",
            "Sends a poll to the Telegram chat",
            "Telegram/Interactive",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "question",
            "Question",
            "Poll question (1-300 characters)",
            VariableType::String,
        );

        node.add_input_pin(
            "options",
            "Options",
            "Array of poll options (2-10 items)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "is_anonymous",
            "Anonymous",
            "Whether the poll is anonymous (default: true)",
            VariableType::Boolean,
        );

        node.add_input_pin(
            "allows_multiple_answers",
            "Multiple Answers",
            "Allow selecting multiple options",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after poll is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let question: String = context.evaluate_pin("question").await?;

        let options: Vec<String> = context.evaluate_pin("options").await?;

        let is_anonymous: bool = context
            .evaluate_pin::<bool>("is_anonymous")
            .await
            .unwrap_or(true);
        let allows_multiple: bool = context
            .evaluate_pin::<bool>("allows_multiple_answers")
            .await
            .unwrap_or(false);

        if options.len() < 2 || options.len() > 10 {
            return Err(flow_like_types::anyhow!("Poll must have 2-10 options"));
        }

        let poll_options: Vec<InputPollOption> =
            options.into_iter().map(InputPollOption::new).collect();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let sent = bot
            .bot
            .send_poll(chat_id, &question, poll_options)
            .is_anonymous(is_anonymous)
            .allows_multiple_answers(allows_multiple)
            .await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Send Location Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendLocationNode;

impl SendLocationNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendLocationNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_location",
            "Send Location",
            "Sends a location point to the Telegram chat",
            "Telegram/Interactive",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "latitude",
            "Latitude",
            "Latitude of the location (-90 to 90)",
            VariableType::Float,
        );

        node.add_input_pin(
            "longitude",
            "Longitude",
            "Longitude of the location (-180 to 180)",
            VariableType::Float,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after location is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let latitude: f64 = context.evaluate_pin("latitude").await?;

        let longitude: f64 = context.evaluate_pin("longitude").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let sent = bot.bot.send_location(chat_id, latitude, longitude).await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Send Venue Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendVenueNode;

impl SendVenueNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendVenueNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_venue",
            "Send Venue",
            "Sends a venue/place with location to the Telegram chat",
            "Telegram/Interactive",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "latitude",
            "Latitude",
            "Latitude of the venue",
            VariableType::Float,
        );

        node.add_input_pin(
            "longitude",
            "Longitude",
            "Longitude of the venue",
            VariableType::Float,
        );

        node.add_input_pin("title", "Title", "Name of the venue", VariableType::String);

        node.add_input_pin(
            "address",
            "Address",
            "Address of the venue",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after venue is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let latitude: f64 = context.evaluate_pin("latitude").await?;

        let longitude: f64 = context.evaluate_pin("longitude").await?;

        let title: String = context.evaluate_pin("title").await?;

        let address: String = context.evaluate_pin("address").await?;

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let sent = bot
            .bot
            .send_venue(chat_id, latitude, longitude, &title, &address)
            .await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

// ============================================================================
// Send Contact Node
// ============================================================================

#[flow_like_catalog_macros::register_node]
#[derive(Default)]
pub struct SendContactNode;

impl SendContactNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SendContactNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "telegram_send_contact",
            "Send Contact",
            "Sends a contact card to the Telegram chat",
            "Telegram/Interactive",
        );
        node.add_icon("/flow/icons/telegram.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Telegram session",
            VariableType::Struct,
        )
        .set_schema::<TelegramSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "phone_number",
            "Phone Number",
            "Contact's phone number",
            VariableType::String,
        );

        node.add_input_pin(
            "first_name",
            "First Name",
            "Contact's first name",
            VariableType::String,
        );

        node.add_input_pin(
            "last_name",
            "Last Name",
            "Optional last name",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continues after contact is sent",
            VariableType::Execution,
        );

        node.add_output_pin(
            "sent_message",
            "Sent Message",
            "Information about the sent message",
            VariableType::Struct,
        )
        .set_schema::<SentMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let session: TelegramSession = context.evaluate_pin("session").await?;

        let phone_number: String = context.evaluate_pin("phone_number").await?;

        let first_name: String = context.evaluate_pin("first_name").await?;

        let last_name: Option<String> = context.evaluate_pin::<String>("last_name").await.ok();

        let bot = get_telegram_bot(context, &session.ref_id).await?;
        let chat_id = session.chat_id()?;

        let mut request = bot.bot.send_contact(chat_id, &phone_number, &first_name);

        if let Some(ln) = last_name {
            request = request.last_name(ln);
        }

        let sent = request.await?;

        let sent_message = SentMessage {
            message_id: sent.id.0.to_string(),
            chat_id: sent.chat.id.0.to_string(),
            date: sent.date.timestamp(),
        };

        context
            .set_pin_value("sent_message", json!(sent_message))
            .await?;

        let exec_out = context.get_pin_by_name("exec_out").await?;
        context.activate_exec_pin_ref(&exec_out).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Telegram functionality requires the 'execute' feature"
        ))
    }
}

//! Telegram and Discord bots that start flow runs on a device.
//!
//! The crate knows nothing of the runtime or the agent: a [`BotHost`] runs flows, keeps the
//! state document and answers the claim gates; a provider connection receives messages and
//! sends answers. [`run`] connects the two under the limits of a device.

pub mod config;
pub mod host;
pub mod limits;
pub mod render;
pub mod reply;
pub mod runner;
pub mod state;

#[cfg(feature = "discord")]
pub mod discord;
#[cfg(feature = "telegram")]
pub mod telegram;

pub use config::{BotProblem, BotSpec, BotToken, Provider};
pub use host::{BotHost, Gate, RunEnd, StreamEvent};
pub use runner::{
    Bot, Connection, Connector, End, Intake, LinkState, Message, Refusal, Resume, Taken, run,
    run_with,
};
pub use state::{BotsState, Revisions};

pub const TELEGRAM: bool = cfg!(feature = "telegram");
pub const DISCORD: bool = cfg!(feature = "discord");

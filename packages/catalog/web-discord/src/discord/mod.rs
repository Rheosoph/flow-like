//! Discord integration for Flow-Like
//!
//! This module provides nodes for interacting with Discord using the serenity library.
//!
//! ## Usage Flow
//! 1. Receive a chat event from Discord sink (via Chat Event node)
//! 2. Use `To Discord Session` node to create a session from `global_session`
//! 3. Use various Discord operation nodes with the session

pub mod channel;
pub mod dm;
pub mod interaction;
pub mod media;
pub mod message;
pub mod poll;
pub mod reaction;
pub mod session;
pub mod user;

pub use interaction::{ButtonResponse, SelectMenuResponse, UserReply};
pub use media::SentAttachment;
pub use poll::{PollAnswerResult, PollReference, PollResults};
#[cfg(feature = "execute")]
pub use session::CachedDiscordClient;
pub use session::DiscordSession;
pub use user::DiscordUser;

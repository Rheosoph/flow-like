//! Telegram integration for Flow-Like
//!
//! This module provides nodes for interacting with Telegram using the teloxide library.
//!
//! ## Usage Flow
//! 1. Receive a chat event from Telegram sink (via Chat Event node)
//! 2. Use `To Telegram Session` node to create a session from `global_session`
//! 3. Use various Telegram operation nodes with the session

pub mod bot;
pub mod business;
pub mod chat;
pub mod commands;
pub mod files;
pub mod forum;
pub mod games;
pub mod gifts;
pub mod inline;
pub mod interaction;
pub mod interactive;
pub mod invite;
pub mod media;
pub mod member;
pub mod message;
pub mod payments;
pub mod poll;
pub mod session;
pub mod stickers;
pub mod stories;
pub mod user;

pub use bot::BotInfo;
pub use business::{BusinessConnection, StarBalance};
pub use commands::{AdminRights, BotCommandInfo};
pub use files::{FileInfo, PhotoInfo, UserProfilePhotosResult};
pub use forum::{ForumTopicInfo, StickerInfo};
pub use games::GameHighScore;
pub use gifts::GiftInfo;
pub use inline::SentWebAppMessageInfo;
pub use interaction::{CallbackResponse, UserReply};
pub use invite::ChatInviteLink;
pub use member::{AdminInfo, ChatMemberInfo};
pub use payments::{InvoiceLink, LabeledPrice, StarTransaction};
pub use poll::{PollReference, PollResults};
#[cfg(feature = "execute")]
pub use session::CachedTelegramBot;
pub use session::TelegramSession;
pub use stickers::{MaskPositionInfo, StickerInfo as StickerInfoFull, StickerSetInfo};
pub use stories::StoryInfo;
pub use user::TelegramUser;

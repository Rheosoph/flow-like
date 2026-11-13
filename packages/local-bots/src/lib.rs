//! Bots that live on a desktop without an account, with their own inbox. One SQLite file holds
//! the bots, their apps and secrets, and the conversations between the signed-out person
//! ([`LOCAL_OWNER_ID`]) and their bots. Every JSON shape is the API's, plus `local: true` on
//! bots and conversations, and cursors, paging, unread counts and read markers follow the API,
//! so the UI reads local and account data alike, as does the history model: every event has a
//! position (`seq`) to read by, "New topic" markers, deleted messages leave tombstones and bots
//! write summaries (segments) of their chats. Secrets are sealed with XChaCha20-Poly1305
//! under a key the host keeps in the OS keychain. [`transfer`] moves a bot into the signed-in
//! account through any host's [`AccountHub`]; each bot's workspace app is made and deleted
//! through the host's [`WorkspaceHost`].

mod bots;
mod cipher;
mod connectors;
mod error;
mod inbox;
mod model;
mod notes;
mod permissions;
mod recovery;
mod room_decisions;
mod schema;
mod skills;
mod store;
mod tasks;
mod tool_grants;
pub mod transfer;
mod validation;
mod workspace;

#[cfg(test)]
mod tests;

pub use error::{Error, Result, codes};
pub use inbox::LocalConversationFile;
pub use model::{
    Bot, BotApp, BotPermission, BotSecretEncryption, BotSecretInfo, Conversation,
    ConversationMember, ConversationPinState, ConversationSegment, ConversationSegmentList,
    CreateBotRequest, CreateConversationRequest, EntriesQuery, ExportedConversation,
    ExportedSecret, InboxEntriesPage, InboxEntry, LocalBotBundle, MarkReadResponse, OpenedSecrets,
    OwnerConversationDeletion, PinnedMessage, PositionQuery, SecretValue, SegmentsQuery,
    SendEntryRequest, StarredMessage, StarredMessagesPage, UpdateBotRequest, WriteSegmentRequest,
};
pub use room_decisions::{RoomDecisionState, RoomWaitOutcome};
pub use skills::SkillOrigin;
pub use store::{ChangeNotifier, KeyError, LocalBotStore, SecretKeyProvider};
pub use tasks::{TaskAuthority, TaskRequest, ToolTurn};
pub use tool_grants::ParkedTurn;
pub use transfer::{
    AccountHub, BotMover, HubBody, HubCall, HubError, HubMethod, MoveError, MoveIssue,
    MoveListener, MoveProgress, MoveReport, MoveStep, MovedBot, MovedConversation,
    WorkspaceSummary,
};
pub use workspace::{WorkspaceBot, WorkspaceHost};

/// The signed-out person: owner of every local bot, sender of their local messages and member
/// of every local conversation.
pub const LOCAL_OWNER_ID: &str = "local";

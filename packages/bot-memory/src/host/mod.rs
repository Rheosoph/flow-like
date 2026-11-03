//! What a host adds to run the engine on a bot's turns: the inbox as the engine's history
//! ([`entries`]), segment publishing ([`publish`]), model plugs ([`models`]), the per-bot session
//! ([`session`]) and the memory tools' dispatcher ([`toolbox`]).

pub mod admin;
pub mod entries;
pub mod models;
pub mod publish;
pub mod session;
pub mod toolbox;

pub use chrono_tz::Tz;
pub use admin::{EvidenceView, FactAdmin, FactView, MAX_LISTED_FACTS};
pub use entries::{EntrySource, HandleReader, InboxReader, StoredEntry};
pub use models::{ModelEmbedder, ModelLlm, RoutedLlm, SwitchEmbedder, SwitchLlm};
pub use publish::{PublishedSegment, SegmentSink, client_id, publish_summaries};
pub use session::{
    AfterTurn, MemorySession, PreparedTurn, SessionOptions, TurnMessage, TurnRequest,
};
pub use toolbox::MemoryToolbox;

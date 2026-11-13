//! A bot's continuous history: per-turn tool-result masking and compaction planning, segment
//! summaries that cite `#seq` positions, a fact memory whose evidence points back into the
//! history, the bot's read tools and cache-stable prompt assembly.
//!
//! Hosts provide the history ([`HistoryReader`], the inbox), a model ([`MemoryLlm`]), an
//! embedder ([`Embedder`]) and a LanceDB connection in the bot's workspace ([`MemoryStores`]).
//! Design: `todo/bots-memory.md`, research: `todo/bots-memory-research.md`.

pub mod compaction;
pub mod engine;
pub mod facts;
pub mod host;
pub mod llm;
pub mod model;
pub mod pointer;
pub mod prompt;
pub mod retrieval;
pub mod spatial;
pub mod store;
pub mod summary;
pub mod telemetry;
pub mod testing;
pub mod text;
pub mod tools;

pub use compaction::{CompactionConfig, CompactionPlan, Trigger, TurnPhase, TurnPlan};
pub use engine::{CompactionInput, CompactionOutcome, DeletionReport, MemoryBusy, MemoryEngine};
pub use facts::{Evidence, Fact, FactKind};
pub use llm::{Embedder, LlmRequest, LlmTask, MemoryLlm};
pub use model::{
    FactSource, HistoryItem, ItemKind, Role, Seq, SeqSpan, Speaker, ToolPart, ToolResult, UiPart,
};
pub use pointer::{PartRef, Pointer};
pub use prompt::{AssembledPrompt, ModelBudget, ModelProfile, PromptInput, assemble};
pub use store::{CompactionState, MemoryStores, StateConflict};
pub use summary::{NowBlock, SegmentSummary};
pub use tools::{Deleted, HistoryReader, ToolOutput, WriteGate};

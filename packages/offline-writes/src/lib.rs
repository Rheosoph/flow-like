pub mod fs;
pub mod outbox;
pub use outbox::{
    BufferedFiles, BufferedTable, BufferingConfig, OperationLookup, OperationSummary, Outbox,
    OutboxOptions, OutboxReader, OutboxStatus, QueueLanes, QueuedOperation,
};

#[cfg(feature = "runtime")]
pub mod files;
#[cfg(feature = "runtime")]
pub mod host;
#[cfg(feature = "runtime")]
mod limits;
#[cfg(feature = "runtime")]
pub mod manager;
#[cfg(feature = "runtime")]
mod table;

#[cfg(feature = "test-support")]
pub use files::FileOverlayParts;
#[cfg(feature = "runtime")]
pub use files::{
    FileBuffering, FileOverlay, FileOverlayOptions, FileRoute, OfflineErrorClassifier, PendingFile,
    table_format_commit,
};
#[cfg(feature = "runtime")]
pub use host::{Connectivity, Observation, OfflineHost, ReplayError, ReplayErrorKind};
#[cfg(feature = "runtime")]
pub use manager::{
    FastForward, RefreshOutcome, TableActivation, TableSetup, TableState, WriteManager,
    WriteManagerOptions,
};
#[cfg(feature = "runtime")]
pub use table::optional_table;

#[cfg(all(test, feature = "runtime"))]
mod tests {
    mod files;
    mod manager;
}

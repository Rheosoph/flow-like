use crate::BufferedTable;
use flow_like_device_protocol::{
    OfflineExpected, OfflineReplayRequest, OfflineReplayResponse, StoragePurpose,
};
use flow_like_storage::{
    lancedb::Table,
    object_store::{ObjectStore, path::Path as ObjectPath},
};
use flow_like_types::authorization::AuthorizationError;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayErrorKind {
    /// Authorization permanently revoked. The manager quarantines the queue
    /// ("Replay authorization denied; queued data remains on this device").
    Denied,
    /// Definitive rejection that may follow a server-side claim. The manager blocks the
    /// head with `code`; the attempt count is kept.
    Rejected,
    /// Definitive rejection that provably happened before any server-side claim. The
    /// manager blocks the head with `code` and resets its attempt count.
    NotClaimed,
    /// Transient: unreachable, 5xx, 429, a transient 409, or 401 while waiting for a
    /// fresh token. The head is unchanged and its lane backs off.
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ReplayError {
    pub kind: ReplayErrorKind,
    /// Machine code stored with a blocked head: "hub_limit", "forbidden",
    /// "endpoint_missing", "subject_mismatch", "invalid", "digest_reused".
    pub code: Option<String>,
    pub message: String,
}

#[async_trait::async_trait]
pub trait OfflineHost: Send + Sync + 'static {
    /// Any error quarantines the scope and fences reads and writes.
    fn authorization_current(&self) -> Result<(), AuthorizationError>;
    /// POST one frozen request. Must not retry internally after the request may have been sent.
    async fn replay(
        &self,
        request: &OfflineReplayRequest,
    ) -> Result<OfflineReplayResponse, ReplayError>;
    /// Object-key prefix of a purpose, ending in '/': "apps/p/storage/". None = not authorized.
    fn location_prefix(&self, purpose: StoragePurpose) -> Option<String>;
    /// The complete cloud table for snapshots, refreshes, probes and key validation.
    /// Ok(None) means confirmed absent.
    async fn remote_table(&self, table: &BufferedTable) -> anyhow::Result<Option<Table>>;
    /// Lazy mirror only: the bucket-relative cloud content store with current credentials.
    async fn remote_objects(&self) -> anyhow::Result<Arc<dyn ObjectStore>> {
        anyhow::bail!("This device keeps no lazy offline mirror")
    }
    /// Cloud inventory of a database root such as "apps/p/storage/db".
    async fn remote_table_names(&self, database: &ObjectPath) -> anyhow::Result<Vec<String>>;
    /// Called after the digest check, before an acknowledged FilePut payload is released.
    /// `revision` is always `OfflineExpected::FileRevision`.
    fn file_acknowledged(
        &self,
        path: &ObjectPath,
        operation_id: &str,
        bytes: &[u8],
        revision: &OfflineExpected,
    ) -> anyhow::Result<()>;
    /// Called before an acknowledged FileDelete is released.
    fn file_deleted(&self, path: &ObjectPath, operation_id: &str) -> anyhow::Result<()>;
    /// A queued file operation was skipped. The host removes any local copy it made of
    /// that operation's bytes.
    fn file_discarded(&self, _path: &ObjectPath, _operation_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
    /// Enqueue, acknowledge, block, skip or quarantine happened.
    fn queue_changed(&self) {}
    /// A mirror download finished, a cached file was evicted, or a table's offline
    /// availability changed.
    fn mirror_changed(&self) {}
}

/// Outcome of a direct cloud call, for the connectivity breaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Observation {
    Succeeded,
    ConnectFailed,
    TimedOut,
}

/// Connectivity breaker consulted by `FileBuffering::WhenOffline` and fast-forward. It
/// follows the hub: hosts open it only after their own reachability check failed.
pub trait Connectivity: Send + Sync {
    fn is_offline(&self) -> bool;
    fn observe(&self, observation: Observation);
}

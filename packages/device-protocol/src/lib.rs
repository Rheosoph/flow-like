//! Shared enrollment and device-authentication wire formats. Signature verification
//! does not consume a challenge or replay ID; the API must do that atomically.

mod archive;
mod inventory;
mod fleet;
mod management;
mod offline;
mod proof;
mod release;
mod recovery;
mod storage;
mod wire;
mod workload;

pub use archive::*;
pub use inventory::*;
pub use fleet::*;
pub use management::*;
pub use offline::*;
pub use proof::*;
pub use release::*;
pub use recovery::*;
pub use storage::*;
pub use wire::*;
pub use workload::*;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("invalid device protocol input: {0}")]
    Invalid(&'static str),
    #[error("device protocol signature verification failed")]
    InvalidSignature,
    #[error("device protocol key does not match the pinned identity")]
    KeyMismatch,
    #[error("device protocol proof is expired or not yet valid")]
    InvalidTime,
    #[error("device protocol proof does not match its request")]
    BindingMismatch,
}

pub type Result<T> = std::result::Result<T, ProtocolError>;

mod artifact;
pub use artifact::*;

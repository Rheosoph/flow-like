//! Device cryptography shared by native agents and browser controllers.

pub mod archive;
pub mod controller;
pub mod fleet;
mod inventory;
pub mod mls;
pub mod mls_store;
pub mod noise;
pub mod prepared_mls;
pub mod recovery;
pub mod vault;
#[cfg(feature = "wasm")]
pub mod wasm;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("invalid cryptographic input: {0}")]
    InvalidInput(&'static str),
    #[error("peer identity does not match the approved key")]
    UntrustedPeer,
    #[error("message belongs to another scope")]
    WrongScope,
    #[error("cryptographic session is closed or incomplete")]
    SessionUnavailable,
    #[error("cryptographic operation failed: {0}")]
    Operation(&'static str),
}

pub type Result<T> = std::result::Result<T, CryptoError>;

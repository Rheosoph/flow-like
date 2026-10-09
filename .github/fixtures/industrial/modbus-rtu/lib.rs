// Compile the production adapter unchanged against PyModbus's serial server.
#[path = "../../../../packages/industrial/src/modbus.rs"]
pub mod modbus;
pub use modbus::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("Modbus server returned exception {0}")]
    Exception(u8),
    #[error("Industrial request timed out")]
    Timeout,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}

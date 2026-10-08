pub mod modbus;
pub use modbus::*;
pub mod genicam;
pub mod opcua;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("Modbus server returned exception {0}")]
    Exception(u8),
    #[error("Sensor request timed out")]
    Timeout,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}

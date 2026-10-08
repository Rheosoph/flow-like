pub mod ads;
pub mod amqp;
pub mod cifx;
pub mod cip;
pub mod ethercat;
pub mod genicam;
pub mod hart;
pub mod iolink;
pub mod iroh;
pub mod kafka;
pub mod modbus;
pub mod nats;
pub mod opcua;
pub mod profibus;
pub mod redis;
pub mod sparkplug;
pub mod zenoh;
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

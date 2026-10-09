// Compile the production adapter and canonical integration test inside Linux so
// the bidirectional GVCP/GVSP UDP traffic stays on the camera's network.
#[path = "../../../../packages/industrial/src/genicam.rs"]
pub mod genicam;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn require(condition: bool, message: &str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}

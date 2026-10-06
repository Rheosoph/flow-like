use std::io;

/// Every variant names the device or stream and the operation that failed.
#[derive(Clone, Debug, thiserror::Error)]
pub enum Error {
    #[error("Could not unlock the controller keys of device {device_id}: {message}")]
    Unlock { device_id: String, message: String },
    #[error("The controller keys of device {device_id} are locked")]
    Locked { device_id: String },
    #[error("The hub refused a tunnel to device {device_id}: {message}")]
    Refused {
        device_id: String,
        status: Option<u16>,
        message: String,
    },
    #[error("Device {device_id} is unreachable: {message}")]
    Unreachable { device_id: String, message: String },
    #[error("The tunnel to device {device_id} closed: {message}")]
    Closed { device_id: String, message: String },
    #[error("The device reset stream {stream_id} ({code}): {message}")]
    Reset {
        stream_id: u32,
        code: String,
        message: String,
    },
    #[error("Invalid device tunnel input: {0}")]
    Invalid(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The original [`Error`] stays reachable through [`io::Error::get_ref`].
impl From<Error> for io::Error {
    fn from(error: Error) -> Self {
        let kind = match &error {
            Error::Reset { .. } => io::ErrorKind::ConnectionReset,
            Error::Closed { .. } => io::ErrorKind::ConnectionAborted,
            Error::Unreachable { .. } => io::ErrorKind::NotConnected,
            Error::Unlock { .. } | Error::Locked { .. } | Error::Refused { .. } => {
                io::ErrorKind::PermissionDenied
            }
            Error::Invalid(_) => io::ErrorKind::InvalidInput,
        };
        io::Error::new(kind, error)
    }
}

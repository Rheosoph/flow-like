//! The crate's error taxonomy.

use crate::gige::proto::gvcp::GvcpStatus;

#[derive(Debug, thiserror::Error)]
pub enum CameraError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("connect timed out")]
    ConnectTimeout,
    #[error("driver is not connected")]
    Disconnected,
    #[error("timed out waiting for acknowledge")]
    Timeout,
    #[error("device control is held by another application")]
    ControlDenied,
    #[error("device control was lost (heartbeat failed)")]
    ControlLost,
    #[error("device answered command {command:#06x} with {status}")]
    Nak { command: u16, status: GvcpStatus },
    #[error("malformed packet: {0}")]
    Protocol(String),
    #[error("device does not support {0}")]
    Unsupported(&'static str),
    #[error("worker thread failed to start: {0}")]
    Spawn(String),
}

#[cfg(feature = "valuable")]
error_valuable!(CameraError, "CameraError", Nak { command, status });

pub type Result<T> = std::result::Result<T, CameraError>;

#[derive(Debug, thiserror::Error)]
pub enum GenicamError {
    #[error("feature '{0}' not found")]
    NotFound(String),
    #[error("node '{0}' does not support this access")]
    WrongType(String),
    #[error("node '{0}' is not accessible this way (access mode)")]
    Access(String),
    #[error("dangling reference '{0}'")]
    Dangling(String),
    #[error("circular reference through '{0}'")]
    Circular(String),
    #[error("formula in '{0}': {1}")]
    Formula(String, String),
    #[error("enumeration '{0}' has no entry '{1}'")]
    NoSuchEntry(String, String),
    #[error("value {value} out of range [{min}, {max}] for '{name}'")]
    OutOfRange {
        name: String,
        value: i64,
        min: i64,
        max: i64,
    },
    #[error("device description: {0}")]
    Xml(String),
    /// A snap produced no frame within its timeout. Distinct from
    /// [`CameraError::Timeout`], which is a control-transaction timeout.
    #[error("no frame arrived within the snap timeout")]
    FrameTimeout,
    #[error(transparent)]
    Camera(#[from] CameraError),
}

#[cfg(feature = "valuable")]
error_valuable!(
    GenicamError,
    "GenicamError",
    OutOfRange {
        name,
        value,
        min,
        max
    }
);

pub type GenicamResult<T> = std::result::Result<T, GenicamError>;

#[cfg(feature = "py")]
impl From<CameraError> for pyo3::PyErr {
    fn from(e: CameraError) -> Self {
        crate::py::CameraException::new_err(e.to_string())
    }
}

#[cfg(feature = "py")]
impl From<GenicamError> for pyo3::PyErr {
    fn from(e: GenicamError) -> Self {
        match e {
            GenicamError::Camera(inner) => inner.into(),
            other => crate::py::GenicamException::new_err(other.to_string()),
        }
    }
}

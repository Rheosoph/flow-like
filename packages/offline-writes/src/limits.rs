use flow_like_device_protocol::{
    OfflineLimits, OfflineReplayRequest, OfflineResource, ProtocolError, format_limit,
};

pub(crate) enum RequestError {
    /// A size limit; the text names the limit.
    TooLarge(String),
    Invalid(ProtocolError),
}

impl From<RequestError> for anyhow::Error {
    fn from(error: RequestError) -> Self {
        match error {
            RequestError::TooLarge(message) => anyhow::anyhow!(message),
            RequestError::Invalid(error) => error.into(),
        }
    }
}

/// `validate_with` with the engine's size texts.
pub(crate) fn validate_request(
    request: &OfflineReplayRequest,
    limits: &OfflineLimits,
) -> Result<(), RequestError> {
    match request.validate_with(limits) {
        Ok(()) => Ok(()),
        Err(ProtocolError::TooLarge {
            what: "offline request",
            limit,
        }) => Err(RequestError::TooLarge(format!(
            "Offline change exceeds the hub's sync limit of {} once encoded; split the logical batch",
            format_limit(limit)
        ))),
        Err(ProtocolError::TooLarge { limit, .. }) => {
            Err(RequestError::TooLarge(match request.resource {
                OfflineResource::Table { .. } => format!(
                    "Offline mutation exceeds {}; split the logical batch",
                    format_limit(limit)
                ),
                OfflineResource::File { .. } => {
                    format!("Offline file payload exceeds {}", format_limit(limit))
                }
            }))
        }
        Err(error) => Err(RequestError::Invalid(error)),
    }
}

/// Replay limits shared by the manager and its file overlays; `set_limits` changes them hot.
#[derive(Clone)]
pub(crate) struct ReplayLimits(std::sync::Arc<std::sync::RwLock<OfflineLimits>>);

impl ReplayLimits {
    pub(crate) fn new(limits: OfflineLimits) -> Self {
        Self(std::sync::Arc::new(std::sync::RwLock::new(limits)))
    }
    pub(crate) fn get(&self) -> OfflineLimits {
        *self
            .0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    pub(crate) fn set(&self, limits: OfflineLimits) {
        *self
            .0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = limits;
    }
}

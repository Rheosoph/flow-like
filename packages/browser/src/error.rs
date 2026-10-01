pub type Result<T, E = BrowserError> = std::result::Result<T, E>;

#[derive(Debug, Clone, thiserror::Error)]
pub enum BrowserError {
    #[error("{message}")]
    NotFound { message: String },
    #[error("Stale element ref '{reference}' — take a new browser snapshot")]
    StaleRef { reference: String },
    #[error("stale element reference: the element is no longer in the current document")]
    StaleElement,
    #[error("the document changed while {method} was running")]
    LoaderChanged { method: String },
    #[error("javascript error: {message}")]
    Javascript { message: String },
    #[error("script was interrupted by a navigation")]
    NavigationInterrupted,
    #[error("script timeout: the script did not finish within {seconds} s")]
    ScriptTimeout { seconds: u64 },
    #[error("unexpected alert open: {{Alert text : {message}}}")]
    DialogOpen {
        kind: crate::types::DialogType,
        message: String,
    },
    #[error("no such alert: no dialog is open on this page")]
    NoDialog,
    #[error("{message}")]
    NoSuchPage { message: String },
    #[error("{message}")]
    NoSuchFrame { message: String },
    #[error("The frame {frame} is moving to another process")]
    FrameInTransit { frame: String },
    #[error("element not interactable: {message}")]
    NotInteractable { message: String },
    #[error("invalid element state: {message}")]
    InvalidElementState { message: String },
    #[error("element click intercepted: {message}")]
    ClickIntercepted { message: String },
    #[error("invalid argument: {message}")]
    InvalidArgument { message: String },
    #[error("Navigation to {url} failed: {error}")]
    NavigationFailed { url: String, error: String },
    #[error("Timed out receiving message from renderer: {seconds:.3}")]
    RendererTimeout { seconds: f64 },
    #[error("{method} got no reply within {timeout_ms} ms")]
    Timeout { method: String, timeout_ms: u64 },
    #[error("The browser connection closed ({reason})")]
    Disconnected { reason: String },
    #[error("The page crashed (target {target_id}); reload it or open a new page")]
    TargetCrashed { target_id: String },
    #[error("The page or frame closed while {method} was running")]
    TargetClosed { method: String },
    #[error("Chrome DevTools command {method} failed ({code}): {message}")]
    Protocol {
        method: String,
        code: i64,
        message: String,
    },
    #[error("Browser events were lost because a waiter fell behind the event log")]
    EventsLost,
    #[error("{message}")]
    Unsupported { message: String },
    #[error("{message}")]
    Launch { message: String },
    #[error("{message}")]
    Connect { message: String },
    #[error("{message}")]
    Install { message: String },
    #[error("{context}: {message}")]
    Io { context: String, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    NoSuchExecutionContext,
    AbortedByNavigation,
    FrameInTransit,
    NodeGone,
    SessionGone,
    Timeout,
    Fatal,
    Other,
}

const SESSION_NOT_FOUND: i64 = -32001;

const NO_SUCH_EXECUTION_CONTEXT: &[&str] = &[
    "Cannot find default execution context",
    "Cannot find context with specified id",
    "uniqueContextId not found",
];

const ABORTED_BY_NAVIGATION: &[&str] = &[
    "Execution context was destroyed.",
    "Inspected target navigated or closed",
    "Not attached to an active page",
];

const NODE_GONE: &[&str] = &[
    "No node found for given backend id",
    "No node with given id found",
    "Node with given id does not belong to the document",
    "Node is detached from document",
];

impl BrowserError {
    pub fn class(&self) -> ErrorClass {
        match self {
            Self::Protocol { code, message, .. } => protocol_class(*code, message),
            Self::LoaderChanged { .. } => ErrorClass::AbortedByNavigation,
            Self::FrameInTransit { .. } => ErrorClass::FrameInTransit,
            Self::StaleElement => ErrorClass::NodeGone,
            Self::TargetClosed { .. } => ErrorClass::SessionGone,
            Self::Timeout { .. } => ErrorClass::Timeout,
            Self::Disconnected { .. } | Self::TargetCrashed { .. } => ErrorClass::Fatal,
            _ => ErrorClass::Other,
        }
    }

    pub fn io(context: impl Into<String>, error: &std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            message: error.to_string(),
        }
    }

    pub fn protocol(method: &str, code: i64, message: impl Into<String>) -> Self {
        Self::Protocol {
            method: method.to_owned(),
            code,
            message: message.into(),
        }
    }
}

fn protocol_class(code: i64, message: &str) -> ErrorClass {
    let mentions = |needles: &[&str]| needles.iter().any(|needle| message.contains(needle));
    if mentions(NO_SUCH_EXECUTION_CONTEXT) {
        ErrorClass::NoSuchExecutionContext
    } else if mentions(ABORTED_BY_NAVIGATION) {
        ErrorClass::AbortedByNavigation
    } else if mentions(NODE_GONE) {
        ErrorClass::NodeGone
    } else if code == SESSION_NOT_FOUND {
        ErrorClass::SessionGone
    } else {
        ErrorClass::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_messages_map_to_chromedriver_classes() {
        let cases = [
            (
                "Cannot find context with specified id",
                ErrorClass::NoSuchExecutionContext,
            ),
            (
                "Execution context was destroyed.",
                ErrorClass::AbortedByNavigation,
            ),
            (
                "Node with given id does not belong to the document",
                ErrorClass::NodeGone,
            ),
            ("Something else", ErrorClass::Other),
        ];
        for (message, class) in cases {
            assert_eq!(
                BrowserError::protocol("DOM.resolveNode", -32000, message).class(),
                class,
                "{message}"
            );
        }
        assert_eq!(
            BrowserError::protocol(
                "Runtime.evaluate",
                -32001,
                "Session with given id not found"
            )
            .class(),
            ErrorClass::SessionGone
        );
    }

    #[test]
    fn non_protocol_variants_have_fixed_classes() {
        assert_eq!(
            BrowserError::LoaderChanged {
                method: "DOM.resolveNode".into()
            }
            .class(),
            ErrorClass::AbortedByNavigation
        );
        assert_eq!(BrowserError::StaleElement.class(), ErrorClass::NodeGone);
        assert_eq!(
            BrowserError::Disconnected {
                reason: "eof".into()
            }
            .class(),
            ErrorClass::Fatal
        );
        assert_eq!(
            BrowserError::ScriptTimeout { seconds: 30 }.class(),
            ErrorClass::Other
        );
    }

    #[test]
    fn dialog_error_text_matches_chromedriver() {
        let error = BrowserError::DialogOpen {
            kind: crate::types::DialogType::Alert,
            message: "hi".into(),
        };
        assert_eq!(
            error.to_string(),
            "unexpected alert open: {Alert text : hi}"
        );
    }
}

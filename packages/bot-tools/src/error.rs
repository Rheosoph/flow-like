use std::fmt;

use serde::Serialize;

pub mod code {
    pub const INVALID_ARGS: &str = "invalid_args";
    pub const UNKNOWN_TOOL: &str = "unknown_tool";
    pub const SCOPE_REFUSED: &str = "scope_refused";
    pub const NO_WORKSPACE: &str = "no_workspace";
    pub const NOT_FOUND: &str = "not_found";
    pub const TOO_LARGE: &str = "too_large";
    pub const NO_EMBEDDER: &str = "no_embedder";
    pub const MODEL_MISMATCH: &str = "model_mismatch";
    pub const UNSUPPORTED_FILE: &str = "unsupported_file";
    pub const HANDLE_MISMATCH: &str = "handle_mismatch";
    pub const NEEDS_APPROVAL: &str = "needs_approval";
    pub const NOT_YET: &str = "not_yet";
    pub const CODE_ERROR: &str = "code_error";
    pub const CODE_TIMEOUT: &str = "code_timeout";
    pub const CODE_OUT_OF_MEMORY: &str = "code_out_of_memory";
    pub const CODE_FUEL_EXHAUSTED: &str = "code_fuel_exhausted";
    pub const CODE_UNAVAILABLE: &str = "code_unavailable";
    pub const SHELL_UNAVAILABLE: &str = "shell_unavailable";
    pub const CANCELLED: &str = "cancelled";
    pub const RATE_LIMITED: &str = "rate_limited";
    pub const FAILED: &str = "failed";
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolError {
    pub code: &'static str,
    pub message: String,
    /// More result fields (a failed run keeps its output), merged into the envelope.
    pub details: Option<serde_json::Value>,
}

pub type ToolResult<T> = Result<T, ToolError>;

impl ToolError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(code::INVALID_ARGS, message)
    }

    pub fn refused(message: impl Into<String>) -> Self {
        Self::new(code::SCOPE_REFUSED, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(code::NOT_FOUND, message)
    }

    pub fn no_workspace(action: &str) -> Self {
        Self::new(
            code::NO_WORKSPACE,
            format!("This bot has no workspace, so it cannot {action}"),
        )
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ToolError {}

impl From<flow_like_types::Error> for ToolError {
    fn from(error: flow_like_types::Error) -> Self {
        Self::new(code::FAILED, format!("{error:#}"))
    }
}

use std::fmt;

/// Error codes. Where the API refuses the same thing, the code is the API's.
pub mod codes {
    pub const INVALID: &str = "INVALID";
    pub const NOT_FOUND: &str = "NOT_FOUND";
    /// Needs an account: a server or device location, end-to-end encryption.
    pub const ACCOUNT_REQUIRED: &str = "ACCOUNT_REQUIRED";
    pub const STORAGE: &str = "STORAGE";
    /// The host could not provide the key secrets are sealed with.
    pub const SECRET_KEY: &str = "SECRET_KEY";
    /// A stored secret does not open with this device's key.
    pub const SECRET_UNREADABLE: &str = "SECRET_UNREADABLE";
    /// The local copy changed after the snapshot a conditional forget names; nothing was deleted.
    pub const LOCAL_CHANGED: &str = "LOCAL_CHANGED";
    pub const BOT_LIMIT_REACHED: &str = "BOT_LIMIT_REACHED";
    /// `flowpilot` is never switched on or off, and a bot that holds it keeps its capabilities.
    pub const BOT_CAPABILITIES_FIXED: &str = "BOT_CAPABILITIES_FIXED";
    /// Assistant and Flux stay; their edits can be undone instead.
    pub const BOT_SYSTEM_UNDELETABLE: &str = "BOT_SYSTEM_UNDELETABLE";
    /// Flux has no workspace yet, as on the account.
    pub const BOT_WORKSPACE_REFUSED: &str = "BOT_WORKSPACE_REFUSED";
    /// The host could not make, find or delete a bot's workspace app or memory.
    pub const BOT_WORKSPACE_FAILED: &str = "BOT_WORKSPACE_FAILED";
    pub const BOT_SECRET_LIMIT_REACHED: &str = "BOT_SECRET_LIMIT_REACHED";
    pub const BOT_APP_LIMIT_REACHED: &str = "BOT_APP_LIMIT_REACHED";
    pub const INBOX_MEMBER_NOT_ALLOWED: &str = "INBOX_MEMBER_NOT_ALLOWED";
    pub const INBOX_MEMBER_LIMIT: &str = "INBOX_MEMBER_LIMIT";
    pub const INBOX_DIRECT_FIXED: &str = "INBOX_DIRECT_FIXED";
    pub const INBOX_MESSAGE_ID_TAKEN: &str = "INBOX_MESSAGE_ID_TAKEN";
    pub const INBOX_TARGET_NOT_FOUND: &str = "INBOX_TARGET_NOT_FOUND";
    pub const INBOX_NOT_YOUR_MESSAGE: &str = "INBOX_NOT_YOUR_MESSAGE";
    /// A message takes at most 40 edits, so a conversation stays small enough to move.
    pub const INBOX_EDIT_LIMIT: &str = "INBOX_EDIT_LIMIT";
    /// A conversation holds at most 50 pinned messages.
    pub const INBOX_PIN_LIMIT: &str = "INBOX_PIN_LIMIT";
    /// Only a bot attaches workspace files, and only from its own workspace.
    pub const INBOX_HANDLE_NOT_ALLOWED: &str = "INBOX_HANDLE_NOT_ALLOWED";
    /// A segment id was written with other content.
    pub const INBOX_SEGMENT_ID_TAKEN: &str = "INBOX_SEGMENT_ID_TAKEN";
    /// A segment lies past the newest position or covers entries its bot doesn't hold.
    pub const INBOX_SEGMENT_RANGE: &str = "INBOX_SEGMENT_RANGE";
    /// A segment's parent is not its bot's higher-level segment covering it.
    pub const INBOX_SEGMENT_PARENT: &str = "INBOX_SEGMENT_PARENT";
    /// A note moved past the version an edit was made against.
    pub const NOTE_CONFLICT: &str = "NOTE_CONFLICT";
}

/// Why a local bot or inbox operation was refused or failed. The message is what the person
/// sees; it never carries a secret value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    code: &'static str,
    message: String,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new(codes::INVALID, message)
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::new(codes::NOT_FOUND, message)
    }

    pub(crate) fn account_required(message: impl Into<String>) -> Self {
        Self::new(codes::ACCOUNT_REQUIRED, message)
    }

    pub(crate) fn storage(message: impl Into<String>) -> Self {
        Self::new(codes::STORAGE, message)
    }

    pub(crate) fn local_changed(message: impl Into<String>) -> Self {
        Self::new(codes::LOCAL_CHANGED, message)
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::storage(format!("Local bot storage failed: {error}"))
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::storage(format!("Local bot data is not valid JSON: {error}"))
    }
}

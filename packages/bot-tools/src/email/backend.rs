use async_trait::async_trait;
use flow_like_types::Bytes;

#[derive(Debug, Clone)]
pub struct MailFile {
    pub filename: String,
    pub content_type: String,
    pub bytes: Bytes,
}

#[derive(Debug, Clone)]
pub struct OutgoingMail {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub text: String,
    pub html: String,
    pub attachments: Vec<MailFile>,
}

#[derive(Debug, Clone)]
pub struct ReplyMail {
    pub thread: String,
    pub text: String,
    pub html: String,
    pub attachments: Vec<MailFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentMail {
    pub message_id: String,
    pub thread: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailFailure {
    pub code: &'static str,
    pub message: String,
}

impl MailFailure {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub mod failure {
    pub const NO_ADDRESS: &str = "mail_no_address";
    pub const LIMIT: &str = "mail_limit";
    pub const REFUSED: &str = "mail_refused";
    pub const UNAVAILABLE: &str = "mail_unavailable";
    pub const NO_THREAD: &str = "mail_no_thread";
}

/// The host's mail service. It sends from the bot's own address, enforces the platform limits
/// and records every sent mail in the bot's mail thread.
#[async_trait]
pub trait MailBackend: Send + Sync {
    async fn send(&self, mail: OutgoingMail) -> Result<SentMail, MailFailure>;
    async fn reply(&self, mail: ReplyMail) -> Result<SentMail, MailFailure>;
}

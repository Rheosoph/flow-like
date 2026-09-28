//! Sink types and enums

use serde::{Deserialize, Serialize};

/// Sink type identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkType {
    /// HTTP/REST API endpoint
    Http,
    /// Incoming webhook from external service
    Webhook,
    /// Cron scheduled trigger
    Cron,
    /// MQTT message broker
    Mqtt,
    /// GitHub repository webhook
    GitHub,
    /// RSS feed polling
    Rss,
    /// Discord bot
    Discord,
    /// Slack bot
    Slack,
    /// Telegram bot
    Telegram,
    /// Email/IMAP polling
    Email,
    /// Email delivered to an address issued by the server
    InboundEmail,
    /// Microsoft Teams bot webhook
    Teams,
    /// Desktop deeplink URL scheme
    Deeplink,
    /// NFC tag scanning
    Nfc,
    /// Geolocation trigger
    Geolocation,
    /// Keyboard shortcut
    Shortcut,
    /// File system watcher
    File,
    /// MCP (Model Context Protocol)
    Mcp,
    /// Web content watcher
    WebWatcher,
    /// Notion database polling
    Notion,
}

impl SinkType {
    /// Get the string identifier for this sink type
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Webhook => "webhook",
            Self::Cron => "cron",
            Self::Mqtt => "mqtt",
            Self::GitHub => "github",
            Self::Rss => "rss",
            Self::Discord => "discord",
            Self::Slack => "slack",
            Self::Telegram => "telegram",
            Self::Email => "email",
            Self::InboundEmail => "inbound_email",
            Self::Teams => "teams",
            Self::Deeplink => "deeplink",
            Self::Nfc => "nfc",
            Self::Geolocation => "geolocation",
            Self::Shortcut => "shortcut",
            Self::File => "file",
            Self::Mcp => "mcp",
            Self::WebWatcher => "web_watcher",
            Self::Notion => "notion",
        }
    }

    /// Check if this sink type is available on the server
    pub fn is_server_available(&self) -> bool {
        matches!(
            self,
            Self::Http
                | Self::Webhook
                | Self::Cron
                | Self::Mqtt
                | Self::GitHub
                | Self::Rss
                | Self::Email
                | Self::InboundEmail
                | Self::Teams
        )
    }

    /// Check if this sink type is available on desktop
    pub fn is_desktop_available(&self) -> bool {
        !matches!(self, Self::InboundEmail | Self::Teams)
    }

    /// Get the availability of this sink type
    pub fn availability(&self) -> SinkAvailability {
        if self.is_server_available() && self.is_desktop_available() {
            SinkAvailability::Both
        } else if self.is_server_available() {
            SinkAvailability::Remote
        } else {
            SinkAvailability::Local
        }
    }
}

impl std::fmt::Display for SinkType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for SinkType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "http" | "api" => Ok(Self::Http),
            "webhook" => Ok(Self::Webhook),
            "cron" => Ok(Self::Cron),
            "mqtt" => Ok(Self::Mqtt),
            "github" => Ok(Self::GitHub),
            "rss" => Ok(Self::Rss),
            "discord" => Ok(Self::Discord),
            "slack" => Ok(Self::Slack),
            "telegram" => Ok(Self::Telegram),
            "email" => Ok(Self::Email),
            "inbound_email" => Ok(Self::InboundEmail),
            "teams" => Ok(Self::Teams),
            "deeplink" => Ok(Self::Deeplink),
            "nfc" => Ok(Self::Nfc),
            "geolocation" => Ok(Self::Geolocation),
            "shortcut" => Ok(Self::Shortcut),
            "file" => Ok(Self::File),
            "mcp" => Ok(Self::Mcp),
            "web_watcher" => Ok(Self::WebWatcher),
            "notion" => Ok(Self::Notion),
            _ => Err(format!("Unknown sink type: {}", s)),
        }
    }
}

/// Where a sink can execute
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SinkAvailability {
    /// Only available on desktop
    Local,
    /// Only available on server
    Remote,
    /// Available on both desktop and server
    Both,
}

impl SinkAvailability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Both => "both",
        }
    }
}

/// Sink registration data - matches the database schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinkRegistration {
    /// Unique identifier for this sink registration
    pub id: String,

    /// The event this sink is for (from the board definition)
    pub event_id: String,

    /// The board containing this event
    pub board_id: String,

    /// App the event belongs to
    pub app_id: String,

    /// Sink type
    pub sink_type: SinkType,

    /// Sink-specific configuration
    pub config: flow_like_types::Value,

    /// Where this sink runs
    pub execution: SinkExecution,

    /// Is the sink currently active?
    pub active: bool,

    /// Auth token for securing HTTP endpoints
    pub auth_token: Option<String>,

    /// For HTTP sinks: the unique path
    pub path: Option<String>,

    /// For HTTP sinks: the method (GET, POST, etc.)
    pub method: Option<String>,

    /// For cron sinks: the expression
    pub cron_expression: Option<String>,

    /// Default payload to merge with incoming data
    pub default_payload: Option<flow_like_types::Value>,

    /// Personal access token for execution
    pub personal_access_token: Option<String>,

    /// OAuth tokens if needed
    pub oauth_tokens: Option<flow_like_types::Value>,
}

/// Where a sink should execute - maps to the Prisma enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SinkExecution {
    /// Only runs on desktop
    Local,
    /// Only runs on server
    #[default]
    Remote,
    /// Runs on both (based on board execution mode)
    Hybrid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_email_is_server_only_and_distinct_from_imap() {
        let inbound: SinkType = "inbound_email".parse().unwrap();
        assert_eq!(inbound, SinkType::InboundEmail);
        assert_eq!(inbound.as_str(), "inbound_email");
        assert_eq!(inbound.availability(), SinkAvailability::Remote);
        assert_eq!("email".parse::<SinkType>().unwrap(), SinkType::Email);
        assert!(SinkType::Email.is_desktop_available());
    }
}

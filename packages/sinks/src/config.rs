//! Sink configuration types

use serde::{Deserialize, Serialize};

/// HTTP sink configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpSinkConfig {
    /// The path for this endpoint (e.g., "/webhook")
    pub path: String,

    /// HTTP method (GET, POST, PUT, PATCH, DELETE)
    pub method: String,

    /// Optional auth token for securing the endpoint
    pub auth_token: Option<String>,

    /// Whether this is a public endpoint (no user auth required)
    #[serde(default)]
    pub public_endpoint: bool,
}

impl Default for HttpSinkConfig {
    fn default() -> Self {
        Self {
            path: "/webhook".to_string(),
            method: "POST".to_string(),
            auth_token: None,
            public_endpoint: false,
        }
    }
}

/// Webhook sink configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookSinkConfig {
    /// The path for this webhook
    pub path: String,

    /// Expected provider (github, stripe, etc.) for signature verification
    pub provider: Option<String>,

    /// Secret for signature verification
    pub secret: Option<String>,
}

/// Scheduled date+time for one-time execution (local time, paired with timezone)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledLocal {
    pub date: String, // "YYYY-MM-DD"
    pub time: String, // "HH:mm"
}

/// Cron sink configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSinkConfig {
    /// Cron expression (e.g., "*/5 * * * *" for every 5 minutes)
    pub expression: String,

    /// Timezone for the cron expression (e.g., "UTC", "America/New_York")
    #[serde(default = "default_timezone")]
    pub timezone: String,

    /// One-time scheduled execution (local date+time, resolved with timezone)
    #[serde(default)]
    pub scheduled_for: Option<ScheduledLocal>,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

impl Default for CronSinkConfig {
    fn default() -> Self {
        Self {
            expression: "0 * * * *".to_string(), // Every hour
            timezone: default_timezone(),
            scheduled_for: None,
        }
    }
}

impl CronSinkConfig {
    pub fn is_one_time(&self) -> bool {
        self.scheduled_for.is_some()
    }

    /// Returns the effective schedule expression.
    /// For one-time schedules, returns an `at()` expression for EventBridge
    /// or a cron expression that would fire at the given time.
    pub fn effective_expression(&self) -> Option<String> {
        if let Some(ref scheduled) = self.scheduled_for {
            Some(format!("at({}T{}:00)", scheduled.date, scheduled.time))
        } else if !self.expression.is_empty() {
            Some(self.expression.clone())
        } else {
            None
        }
    }
}

/// MQTT sink configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttSinkConfig {
    /// MQTT broker URL
    pub broker_url: String,

    /// Topic to subscribe to
    pub topic: String,

    /// Optional username
    pub username: Option<String>,

    /// Optional password
    pub password: Option<String>,

    /// QoS level (0, 1, or 2)
    #[serde(default = "default_qos")]
    pub qos: u8,
}

fn default_qos() -> u8 {
    1
}

/// RSS sink configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssSinkConfig {
    /// RSS feed URL
    pub feed_url: String,

    /// Polling interval in seconds
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
}

fn default_poll_interval() -> u64 {
    300 // 5 minutes
}

/// Unified sink configuration enum
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SinkConfig {
    Http(HttpSinkConfig),
    Webhook(WebhookSinkConfig),
    Cron(CronSinkConfig),
    Mqtt(MqttSinkConfig),
    Rss(RssSinkConfig),
}

impl SinkConfig {
    /// Get the sink type string
    pub fn sink_type(&self) -> &'static str {
        match self {
            Self::Http(_) => "http",
            Self::Webhook(_) => "webhook",
            Self::Cron(_) => "cron",
            Self::Mqtt(_) => "mqtt",
            Self::Rss(_) => "rss",
        }
    }
}

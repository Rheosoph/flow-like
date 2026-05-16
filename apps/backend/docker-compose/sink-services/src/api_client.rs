use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronScheduleInfo {
    pub id: String,
    pub event_id: String,
    pub cron_expression: String,
    pub enabled: bool,
    pub last_triggered: Option<chrono::DateTime<chrono::Utc>>,
    pub next_trigger: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug)]
pub struct ApiClient {
    client: Client,
    base_url: String,
    jwt: String,
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>, jwt: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
            jwt: jwt.into(),
        }
    }

    pub async fn trigger_event(
        &self,
        event_id: &str,
        sink_type: &str,
        payload: Option<serde_json::Value>,
    ) -> Result<(), ApiError> {
        let url = format!("{}/api/v1/sink/trigger/async", self.base_url);

        let body = serde_json::json!({
            "event_id": event_id,
            "sink_type": sink_type,
            "payload": payload,
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.jwt))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ApiError::Request(e.to_string()))?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            Err(ApiError::Response {
                status: status.as_u16(),
                message: text,
            })
        }
    }

    pub async fn get_cron_schedules(&self) -> Result<Vec<CronScheduleInfo>, ApiError> {
        let url = format!("{}/api/v1/sink/schedules", self.base_url);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.jwt))
            .send()
            .await
            .map_err(|e| ApiError::Request(e.to_string()))?;

        if response.status().is_success() {
            let schedules: Vec<CronScheduleInfo> = response
                .json()
                .await
                .map_err(|e| ApiError::Parse(e.to_string()))?;
            Ok(schedules)
        } else {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            Err(ApiError::Response {
                status: status.as_u16(),
                message: text,
            })
        }
    }

    /// Trigger a sink event via the async trigger endpoint
    pub async fn trigger_sink(
        &self,
        event_id: &str,
        sink_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), ApiError> {
        let url = format!("{}/api/v1/sink/trigger/async", self.base_url);

        let body = serde_json::json!({
            "event_id": event_id,
            "sink_type": sink_type,
            "payload": payload,
        });

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.jwt))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ApiError::Request(e.to_string()))?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            Err(ApiError::Response {
                status: status.as_u16(),
                message: text,
            })
        }
    }

    /// Get bot configurations grouped by token
    /// Returns bots with their tokens and associated event handlers
    pub async fn get_bot_configs(&self, sink_type: &str) -> Result<Vec<BotConfig>, ApiError> {
        let url = format!("{}/api/v1/sink/bots?sink_type={}", self.base_url, sink_type);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.jwt))
            .send()
            .await
            .map_err(|e| ApiError::Request(e.to_string()))?;

        if response.status().is_success() {
            let configs: Vec<BotConfig> = response
                .json()
                .await
                .map_err(|e| ApiError::Parse(e.to_string()))?;
            Ok(configs)
        } else {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            Err(ApiError::Response {
                status: status.as_u16(),
                message: text,
            })
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    pub bot_id: String,
    pub token: String,
    pub sink_type: String,
    pub handlers: Vec<BotHandler>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotHandler {
    pub event_id: String,
    pub config: serde_json::Value,
}

#[derive(Debug)]
pub enum ApiError {
    Request(String),
    Response { status: u16, message: String },
    Parse(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Request(e) => write!(f, "Request error: {}", e),
            ApiError::Response { status, message } => {
                write!(f, "Response error ({}): {}", status, message)
            }
            ApiError::Parse(e) => write!(f, "Parse error: {}", e),
        }
    }
}

impl std::error::Error for ApiError {}

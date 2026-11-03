#![cfg_attr(not(any(feature = "discord", feature = "telegram")), allow(dead_code))]

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

    /// Active events of a bot sink type, grouped into one bot per token.
    pub async fn get_bot_configs(&self, sink_type: &str) -> Result<Vec<BotConfig>, ApiError> {
        let url = sink_configs_url(&self.base_url, sink_type);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.jwt))
            .send()
            .await
            .map_err(|e| ApiError::Request(e.to_string()))?;

        if response.status().is_success() {
            let sinks: Vec<SinkConfigInfo> = response
                .json()
                .await
                .map_err(|e| ApiError::Parse(e.to_string()))?;
            Ok(group_bots(sink_type, sinks))
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

const SINK_CONFIGS_PATH: &str = "/api/v1/sink/configs";

fn sink_configs_url(base_url: &str, sink_type: &str) -> String {
    format!("{base_url}{SINK_CONFIGS_PATH}?sink_type={sink_type}")
}

/// One row of `GET /sink/configs`: the event's config is stored as `{"base64": <config bytes>}`.
#[derive(Debug, Deserialize)]
struct SinkConfigInfo {
    event_id: String,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    config: Option<serde_json::Value>,
}

impl SinkConfigInfo {
    fn settings(&self) -> Option<serde_json::Map<String, serde_json::Value>> {
        use base64::Engine as _;
        let encoded = self.config.as_ref()?.get("base64")?.as_str()?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()?;
        match serde_json::from_slice(&bytes).ok()? {
            serde_json::Value::Object(settings) => Some(settings),
            _ => None,
        }
    }
}

/// Events that share a bot token share one bot. Events without a readable token are left out.
fn group_bots(sink_type: &str, sinks: Vec<SinkConfigInfo>) -> Vec<BotConfig> {
    let mut bots: std::collections::BTreeMap<String, BotConfig> = Default::default();
    for sink in sinks.into_iter().filter(|sink| sink.active) {
        let Some(mut settings) = sink.settings() else {
            continue;
        };
        let Some(token) = settings
            .remove("bot_token")
            .and_then(|token| token.as_str().map(|token| token.trim().to_owned()))
            .filter(|token| !token.is_empty())
        else {
            continue;
        };
        bots.entry(token.clone())
            .or_insert_with(|| BotConfig {
                bot_id: crate::storage::RedisStorage::hash_token(&token),
                token,
                sink_type: sink_type.to_owned(),
                handlers: Vec::new(),
            })
            .handlers
            .push(BotHandler {
                event_id: sink.event_id,
                config: serde_json::Value::Object(settings),
            });
    }
    bots.into_values().collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use serde_json::json;

    fn sink(event_id: &str, active: bool, settings: serde_json::Value) -> SinkConfigInfo {
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&settings).unwrap());
        SinkConfigInfo {
            event_id: event_id.into(),
            active,
            config: Some(json!({ "base64": encoded })),
        }
    }

    #[test]
    fn bots_are_read_from_the_route_the_api_serves() {
        assert_eq!(
            sink_configs_url("http://api:8080", "telegram"),
            "http://api:8080/api/v1/sink/configs?sink_type=telegram"
        );
        let routes = include_str!("../../../../../packages/api/src/routes/sink/mod.rs");
        assert!(routes.contains(".route(\"/configs\", get(trigger::get_sink_configs))"));
        assert!(!routes.contains("\"/bots\""));
    }

    #[test]
    fn events_on_one_token_share_one_bot() {
        let bots = group_bots(
            "telegram",
            vec![
                sink("evt_a", true, json!({"bot_token": "1:abc", "chat_id": 7})),
                sink(
                    "evt_b",
                    true,
                    json!({"bot_token": " 1:abc ", "command": "/go"}),
                ),
                sink("evt_c", true, json!({"bot_token": "2:def"})),
            ],
        );
        assert_eq!(bots.len(), 2);
        let shared = bots.iter().find(|bot| bot.token == "1:abc").unwrap();
        assert_eq!(shared.sink_type, "telegram");
        assert_eq!(
            shared
                .handlers
                .iter()
                .map(|handler| handler.event_id.as_str())
                .collect::<Vec<_>>(),
            ["evt_a", "evt_b"]
        );
        assert_eq!(shared.handlers[0].config, json!({"chat_id": 7}));
        assert_ne!(
            shared.bot_id,
            bots.iter().find(|bot| bot.token == "2:def").unwrap().bot_id
        );
    }

    #[test]
    fn a_bot_id_is_stable_and_never_the_token() {
        let rows = || vec![sink("evt_a", true, json!({"bot_token": "1:abc"}))];
        let first = group_bots("telegram", rows());
        let again = group_bots("telegram", rows());
        assert_eq!(first[0].bot_id, again[0].bot_id);
        assert!(!first[0].bot_id.contains("abc"));
    }

    #[test]
    fn inactive_events_and_events_without_a_token_start_no_bot() {
        let mut unreadable = sink("evt_d", true, json!({}));
        unreadable.config = Some(json!({"base64": "***"}));
        let bots = group_bots(
            "discord",
            vec![
                sink("evt_a", false, json!({"bot_token": "tok"})),
                sink("evt_b", true, json!({"bot_token": "  "})),
                sink("evt_c", true, json!({"chat_id": 1})),
                unreadable,
                SinkConfigInfo {
                    event_id: "evt_e".into(),
                    active: true,
                    config: None,
                },
            ],
        );
        assert!(bots.is_empty());
    }
}

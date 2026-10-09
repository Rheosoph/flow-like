use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};

pub type Version = (u32, u32, u32);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelInfo {
    pub bit_id: String,
    pub name: String,
    pub description: String,
    pub provider_name: Option<String>,
    pub api_surface: Option<String>,
    pub model_id: Option<String>,
    pub context_length: Option<u64>,
    pub vector_length: Option<u64>,
    pub languages: Vec<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DatabaseAction {
    CreateBranch { name: String },
    DeleteBranch { name: String },
    CreateTag { name: String },
    UpdateTag { name: String },
    DeleteTag { name: String },
    Restore,
    Snapshot { name: Option<String> },
    Cleanup { older_than_days: u64 },
    Clone { name: String },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InvokeBoardRequest {
    pub node_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Version>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_tokens: Option<BTreeMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_variables: Option<BTreeMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_state: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct InvokeEventRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_tokens: Option<BTreeMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_variables: Option<BTreeMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_trigger: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AsyncInvokeResult {
    pub run_id: String,
    pub status: String,
    pub poll_token: String,
    pub backend: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutionEvent {
    pub sequence: i32,
    pub event_type: String,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PollResponse {
    pub run_id: String,
    pub status: String,
    pub progress: i32,
    pub current_step: Option<String>,
    pub error: Option<String>,
    pub events: Vec<ExecutionEvent>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

impl PollResponse {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status.to_ascii_uppercase().as_str(),
            "COMPLETED" | "FAILED" | "CANCELLED" | "TIMEOUT"
        )
    }
    pub fn last_sequence(&self) -> Option<i32> {
        self.events.iter().map(|event| event.sequence).max()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunStatus {
    pub run_id: String,
    pub board_id: String,
    pub event_id: Option<String>,
    pub status: String,
    pub mode: String,
    pub progress: i32,
    pub current_step: Option<String>,
    pub error: Option<String>,
    pub input_payload_len: i64,
    pub output_payload_len: i64,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CancelRunResult {
    pub run_id: String,
    pub status: String,
    pub cancelled: bool,
}

#[derive(Clone, Debug)]
pub struct PollOptions {
    pub after_sequence: i32,
    pub timeout: u64,
}
impl Default for PollOptions {
    fn default() -> Self {
        Self {
            after_sequence: -1,
            timeout: 10,
        }
    }
}

#[derive(Clone, Debug)]
pub struct WaitOptions {
    pub timeout: Duration,
    pub poll_timeout: u64,
}
impl Default for WaitOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(300),
            poll_timeout: 10,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SignedFile {
    pub prefix: String,
    pub url: Option<String>,
    pub error: Option<String>,
    pub method: Option<String>,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DatabaseSelector {
    pub scope: Option<String>,
    pub branch: Option<String>,
    pub version: Option<u64>,
    pub tag: Option<String>,
    pub read_only: Option<bool>,
}

impl DatabaseSelector {
    pub fn options(&self) -> crate::RequestOptions {
        let mut options = crate::RequestOptions::default();
        for (key, value) in [
            ("scope", &self.scope),
            ("branch", &self.branch),
            ("tag", &self.tag),
        ] {
            if let Some(value) = value {
                options = options.query(key, value);
            }
        }
        if let Some(version) = self.version {
            options = options.query("version", version);
        }
        if let Some(read_only) = self.read_only {
            options = options.query("read_only", read_only);
        }
        options
    }
}

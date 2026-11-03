use std::collections::BTreeMap;

use flow_like_bot_tools::{Access, AppGrant, Scope, ToolOutcome};
use flow_like_types::base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like_types::data_classification::DataClassification;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const IMAGES_FIELD: &str = "_flowpilot_image_urls";
const JOB_HASH_DOMAIN: &str = "flow-like:bot-tool-job:v1:";

/// What the API sends to the executor's `POST /tool`: the signed token and the job as the exact
/// JSON text the token's `dispatch_hash` covers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolJobEnvelope {
    pub executor_jwt: String,
    pub job: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolReply {
    pub output: String,
    pub cpu_ms: u64,
}

/// The digest an executor token binds to one job.
pub fn job_hash(job: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(JOB_HASH_DOMAIN.as_bytes());
    hasher.update(job.as_bytes());
    hasher.finalize().to_hex().to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobApp {
    pub app_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    pub files: bool,
    pub tables: bool,
    /// Attributes of the owner's role in this app; `None` reads every column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_attributes: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobScope {
    pub storage: bool,
    pub data_connections: bool,
    pub classification: DataClassification,
    pub apps: Vec<JobApp>,
}

impl JobScope {
    pub fn to_scope(&self, workspace: &str) -> Scope {
        Scope {
            workspace: Some(workspace.to_string()),
            apps: self
                .apps
                .iter()
                .filter(|app| app.files || app.tables)
                .map(|app| AppGrant {
                    app_id: app.app_id.clone(),
                    access: Access::Read,
                    alias: app.alias.clone(),
                })
                .collect(),
            storage: self.storage,
            data_connections: self.data_connections,
            classification: self.classification,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobModels {
    /// The Bit the bot answers with, as the profile holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolJob {
    pub tool: String,
    pub args: Value,
    pub owner: String,
    /// The bot the turn belongs to; the executor token must carry the same bot.
    #[serde(default)]
    pub bot_id: String,
    pub turn_id: String,
    pub workspace: String,
    pub scope: JobScope,
    /// One shared credential per app, as the API scoped it: read and write on the workspace,
    /// read on every other app.
    pub credentials: BTreeMap<String, Value>,
    #[serde(default)]
    pub models: JobModels,
    #[serde(default)]
    pub allow_expensive_documents: bool,
}

/// The text a host returns to the model for a finished tool: the envelope, with the outcome's
/// images moved into the field the tool loop turns into image parts of the result message.
pub fn outcome_output(outcome: &ToolOutcome) -> String {
    let mut envelope = outcome.envelope();
    if !outcome.images.is_empty()
        && let Some(object) = envelope.as_object_mut()
    {
        let images: Vec<Value> = outcome
            .images
            .iter()
            .map(|image| json!({ "data": STANDARD.encode(&image.data), "media_type": image.mime }))
            .collect();
        object.insert(IMAGES_FIELD.to_string(), Value::Array(images));
    }
    envelope.to_string()
}

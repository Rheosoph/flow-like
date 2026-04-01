use chrono::{DateTime, Utc};
use flow_like_catalog_core::FlowPath;
use flow_like_types::Value;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::templates::MatchResult;

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub enum ArtifactType {
    Screenshot,
    Pdf,
    Template,
    DomSnapshot,
    AxSnapshot,
    Download,
    DiagnosticsBundle,
    Video,
    Har,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct ArtifactRef {
    pub artifact_id: String,
    pub artifact_type: ArtifactType,
    pub path: FlowPath,
    pub mime_type: String,
    pub size_bytes: Option<u64>,
    pub created_at: DateTime<Utc>,
    pub metadata: std::collections::HashMap<String, String>,
}

impl ArtifactRef {
    pub fn new(
        artifact_id: impl Into<String>,
        artifact_type: ArtifactType,
        path: FlowPath,
        mime_type: impl Into<String>,
    ) -> Self {
        Self {
            artifact_id: artifact_id.into(),
            artifact_type,
            path,
            mime_type: mime_type.into(),
            size_bytes: None,
            created_at: Utc::now(),
            metadata: std::collections::HashMap::new(),
        }
    }

    pub fn with_size(mut self, size_bytes: u64) -> Self {
        self.size_bytes = Some(size_bytes);
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub message: String,
    pub source: Option<String>,
    pub details: Option<Value>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogEntry {
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            level: LogLevel::Info,
            message: message.into(),
            source: None,
            details: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            timestamp: Utc::now(),
            level: LogLevel::Error,
            message: message.into(),
            source: None,
            details: None,
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
pub enum DiagnosticsLevel {
    #[default]
    Light,
    Full,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct DiagnosticsBundle {
    pub bundle_id: String,
    pub timestamp: DateTime<Utc>,
    pub level: DiagnosticsLevel,
    pub screenshot: Option<ArtifactRef>,
    pub dom_snapshot: Option<ArtifactRef>,
    pub ax_snapshot: Option<ArtifactRef>,
    pub logs: Vec<LogEntry>,
    pub template_hits: Vec<MatchResult>,
    pub error: Option<String>,
    pub context: DiagnosticsContext,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct DiagnosticsContext {
    pub page_url: Option<String>,
    pub window_title: Option<String>,
    pub step_name: Option<String>,
    pub selector_attempted: Option<String>,
    pub action_type: Option<String>,
}

impl DiagnosticsBundle {
    pub fn new(bundle_id: impl Into<String>, level: DiagnosticsLevel) -> Self {
        Self {
            bundle_id: bundle_id.into(),
            timestamp: Utc::now(),
            level,
            screenshot: None,
            dom_snapshot: None,
            ax_snapshot: None,
            logs: Vec::new(),
            template_hits: Vec::new(),
            error: None,
            context: DiagnosticsContext::default(),
        }
    }

    pub fn with_screenshot(mut self, screenshot: ArtifactRef) -> Self {
        self.screenshot = Some(screenshot);
        self
    }

    pub fn with_dom_snapshot(mut self, snapshot: ArtifactRef) -> Self {
        self.dom_snapshot = Some(snapshot);
        self
    }

    pub fn with_ax_snapshot(mut self, snapshot: ArtifactRef) -> Self {
        self.ax_snapshot = Some(snapshot);
        self
    }

    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }

    pub fn add_log(&mut self, entry: LogEntry) {
        self.logs.push(entry);
    }

    pub fn add_template_hit(&mut self, result: MatchResult) {
        self.template_hits.push(result);
    }
}

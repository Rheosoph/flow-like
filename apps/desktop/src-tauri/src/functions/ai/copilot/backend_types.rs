//! Backend selection, model metadata, and capability types.

use super::tool_policy::specialist_tool_policy;
use flow_like::{
    copilot::CopilotScope,
    flow::copilot::{WebResearchCapability, tool_spec::RESEARCH_AGENT_TOOL},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlowPilotAgentBackendKind {
    GithubCopilot,
    Codex,
    ClaudeCode,
}

impl FlowPilotAgentBackendKind {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "copilot" | "github" | "github-copilot" | "github_copilot" => Some(Self::GithubCopilot),
            "codex" | "openai-codex" | "openai_codex" => Some(Self::Codex),
            "claude" | "claude-code" | "claude_code" => Some(Self::ClaudeCode),
            _ => None,
        }
    }

    pub(super) fn from_model_prefix(value: &str) -> Option<(Self, &str)> {
        for (prefix, backend) in [
            ("copilot:", Self::GithubCopilot),
            ("github-copilot:", Self::GithubCopilot),
            ("codex:", Self::Codex),
            ("claude-code:", Self::ClaudeCode),
            ("claude:", Self::ClaudeCode),
        ] {
            if let Some(model_id) = value.strip_prefix(prefix) {
                return Some((backend, model_id));
            }
        }

        None
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::GithubCopilot => "GitHub Copilot",
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
        }
    }

    pub(super) fn cli_name(self) -> &'static str {
        match self {
            Self::GithubCopilot => "copilot",
            Self::Codex => "codex",
            Self::ClaudeCode => "claude",
        }
    }

    pub(super) fn env_path_var(self) -> &'static str {
        match self {
            Self::GithubCopilot => "COPILOT_CLI_PATH",
            Self::Codex => "CODEX_CLI_PATH",
            Self::ClaudeCode => "CLAUDE_CODE_CLI_PATH",
        }
    }

    /// How the global orchestrator on this backend reaches the public web. Specialists never do.
    pub(super) fn web_research(self) -> WebResearchCapability {
        match self {
            Self::Codex | Self::ClaudeCode => WebResearchCapability::Native,
            Self::GithubCopilot => WebResearchCapability::Delegated,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FlowPilotChatBackend {
    Bits,
    Agent(FlowPilotAgentBackendKind),
}

#[derive(Debug, Clone)]
pub(super) struct FlowPilotModelSelection {
    pub(super) backend: FlowPilotChatBackend,
    pub(super) model_id: Option<String>,
}

impl FlowPilotModelSelection {
    pub(super) fn parse(model_id: Option<String>) -> Self {
        let Some(model_id) = model_id else {
            return Self {
                backend: FlowPilotChatBackend::Bits,
                model_id: None,
            };
        };

        if let Some((backend, stripped_model_id)) =
            FlowPilotAgentBackendKind::from_model_prefix(&model_id)
        {
            return Self {
                backend: FlowPilotChatBackend::Agent(backend),
                model_id: Some(stripped_model_id.to_string()),
            };
        }

        Self {
            backend: FlowPilotChatBackend::Bits,
            model_id: Some(model_id),
        }
    }
}

/// One model-specific reasoning-effort choice discovered from the backend runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningEffortOption {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Model info returned by any local FlowPilot agent backend. Reasoning capabilities
/// stay model-specific because accounts, policies, and installed runtimes can expose
/// different choices even within the same provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopilotModelInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub supported_reasoning_efforts: Vec<ReasoningEffortOption>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<String>,
    #[serde(default)]
    pub is_default: bool,
}

impl CopilotModelInfo {
    pub(super) fn basic(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            supported_reasoning_efforts: Vec::new(),
            default_reasoning_effort: None,
            is_default: false,
        }
    }
}

/// Auth status returned from GitHub Copilot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotAuthStatus {
    pub authenticated: bool,
    pub login: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowPilotBackendStatus {
    pub backend: FlowPilotAgentBackendKind,
    pub label: String,
    pub available: bool,
    pub running: bool,
    pub executable: Option<String>,
    pub message: Option<String>,
    pub transport: FlowPilotAgentTransportKind,
    pub capabilities: FlowPilotAgentCapabilitySet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlowPilotAgentTransportKind {
    DirectSdkTools,
    Mcp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowPilotAgentCapabilitySet {
    pub prompt_source: String,
    pub tool_protocol: FlowPilotAgentTransportKind,
    pub tool_names: Vec<String>,
}

impl FlowPilotAgentCapabilitySet {
    pub(super) fn shared_for(
        scope: CopilotScope,
        has_board: bool,
        has_graph_context: bool,
    ) -> Self {
        let mut tool_names = specialist_tool_policy(scope, has_board, has_graph_context)
            .into_iter()
            .collect::<Vec<_>>();
        tool_names.sort_unstable();

        Self {
            prompt_source: "flow_like::copilot::prompts".to_string(),
            tool_protocol: FlowPilotAgentTransportKind::DirectSdkTools,
            tool_names: tool_names.into_iter().map(str::to_string).collect(),
        }
    }

    pub(super) fn add_global_orchestrator_tools(&mut self) {
        self.tool_names.push(RESEARCH_AGENT_TOOL.to_string());
        self.tool_names.sort_unstable();
        self.tool_names.dedup();
    }

    pub(super) fn for_surface(
        scope: CopilotScope,
        has_board: bool,
        has_graph_context: bool,
        global_orchestrator: bool,
    ) -> Self {
        let mut capabilities = Self::shared_for(scope, has_board, has_graph_context);
        if global_orchestrator {
            capabilities.add_global_orchestrator_tools();
        }
        capabilities
    }

    pub(super) fn for_status(transport: FlowPilotAgentTransportKind) -> Self {
        let mut capabilities = Self::shared_for(CopilotScope::Both, true, true);
        capabilities.tool_protocol = transport;
        capabilities
    }
}

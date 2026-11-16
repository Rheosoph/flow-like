//! Backend implementations, startup options, and provider lifecycle.

use super::backend_types::{
    CopilotAuthStatus, CopilotModelInfo, FlowPilotAgentBackendKind, FlowPilotAgentCapabilitySet,
    FlowPilotAgentTransportKind, FlowPilotBackendStatus, ReasoningEffortOption,
};
use super::cli_auth::{probe_external_agent_auth, probe_external_agent_cli};
use super::cli_resolution::{external_agent_cli_resolution_failure, find_cli_resolution};
use super::client_pool::{
    COPILOT_CLIENT, COPILOT_START_GATE, COPILOT_START_OPTIONS, EXTERNAL_AGENT_BACKENDS,
    NESTED_COPILOT_POOL, TOP_LEVEL_COPILOT_POOL, build_and_start_copilot_client,
};
use super::model_catalog::{
    codex_models_with_configured_default, list_claude_models_via_control_protocol,
    list_codex_models_via_app_server, reasoning_effort_display_name,
};
use super::provider_errors::actionable_external_agent_failure;
use super::runtime::{SDK_CHAT_ABORT_TIMEOUT, SDK_CONTROL_RPC_TIMEOUT};
use async_trait::async_trait;
use std::sync::Arc;
use tauri::AppHandle;

#[derive(Debug, Clone)]
pub(super) struct FlowPilotBackendStartOptions {
    pub(super) use_stdio: bool,
    pub(super) cli_url: Option<String>,
    pub(super) app_handle: Option<AppHandle>,
}

#[async_trait]
pub(super) trait FlowPilotAgentBackend: Send + Sync {
    fn kind(&self) -> FlowPilotAgentBackendKind;
    async fn start(&self, options: FlowPilotBackendStartOptions) -> Result<(), String>;
    async fn stop(&self) -> Result<(), String>;
    async fn is_running(&self) -> Result<bool, String>;
    async fn list_models(
        &self,
        app_handle: Option<&AppHandle>,
    ) -> Result<Vec<CopilotModelInfo>, String>;
    async fn get_auth_status(
        &self,
        app_handle: Option<&AppHandle>,
    ) -> Result<CopilotAuthStatus, String>;
    async fn status(&self, app_handle: Option<&AppHandle>) -> FlowPilotBackendStatus {
        let kind = self.kind();
        let executable = find_cli_resolution(kind, app_handle)
            .map(|resolution| resolution.executable.display().to_string());
        let available = executable.is_some();
        let running = self.is_running().await.unwrap_or(false);

        FlowPilotBackendStatus {
            backend: kind,
            label: kind.label().to_string(),
            available,
            running,
            executable,
            message: None,
            transport: FlowPilotAgentTransportKind::DirectSdkTools,
            capabilities: FlowPilotAgentCapabilitySet::for_status(
                FlowPilotAgentTransportKind::DirectSdkTools,
            ),
        }
    }
}

struct GithubCopilotBackend;

struct ExternalCodeAgentBackend {
    kind: FlowPilotAgentBackendKind,
}

pub(super) fn agent_backend(kind: FlowPilotAgentBackendKind) -> Box<dyn FlowPilotAgentBackend> {
    match kind {
        FlowPilotAgentBackendKind::GithubCopilot => Box::new(GithubCopilotBackend),
        FlowPilotAgentBackendKind::Codex | FlowPilotAgentBackendKind::ClaudeCode => {
            Box::new(ExternalCodeAgentBackend { kind })
        }
    }
}

#[async_trait]
impl FlowPilotAgentBackend for GithubCopilotBackend {
    fn kind(&self) -> FlowPilotAgentBackendKind {
        FlowPilotAgentBackendKind::GithubCopilot
    }

    async fn start(&self, options: FlowPilotBackendStartOptions) -> Result<(), String> {
        let _start_permit =
            tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, COPILOT_START_GATE.acquire())
                .await
                .map_err(|_| {
                    "Timed out waiting for Copilot startup already in progress".to_string()
                })?
                .map_err(|_| "Copilot startup gate was closed".to_string())?;
        if COPILOT_CLIENT.lock().await.is_some() {
            return Ok(());
        }
        let client = Arc::new(build_and_start_copilot_client(&options).await?);

        {
            let mut opts = COPILOT_START_OPTIONS.lock().await;
            *opts = Some(options);
        }
        COPILOT_CLIENT.lock().await.replace(client);

        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        let client = {
            let mut guard = COPILOT_CLIENT.lock().await;
            guard.take()
        };
        // Clear before draining: a checkout that reads options after this point fails fast, and
        // one that read them earlier is rejected by the pool's drain epoch when it registers.
        COPILOT_START_OPTIONS.lock().await.take();
        // Both pools, or backend stop leaves live CLI processes behind that nothing ever reaps.
        let mut nested_clients = NESTED_COPILOT_POOL.drain();
        nested_clients.extend(TOP_LEVEL_COPILOT_POOL.drain());

        let mut errors: Vec<String> = Vec::new();
        if let Some(client) = client {
            let stop_errors = match tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, client.stop())
                .await
            {
                Ok(errors) => errors,
                Err(_) => {
                    let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, client.force_stop()).await;
                    errors.push("main: graceful stop timed out; client force-stopped".to_string());
                    Vec::new()
                }
            };
            if !stop_errors.is_empty() {
                errors.push(format!("{:?}", stop_errors));
            }
        }
        for client in nested_clients {
            let stop_errors = match tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, client.stop())
                .await
            {
                Ok(errors) => errors,
                Err(_) => {
                    let _ = tokio::time::timeout(SDK_CHAT_ABORT_TIMEOUT, client.force_stop()).await;
                    errors
                        .push("nested: graceful stop timed out; client force-stopped".to_string());
                    Vec::new()
                }
            };
            if !stop_errors.is_empty() {
                errors.push(format!("nested: {:?}", stop_errors));
            }
        }
        if !errors.is_empty() {
            return Err(format!(
                "Failed to stop Copilot client: {}",
                errors.join("; ")
            ));
        }

        Ok(())
    }

    async fn is_running(&self) -> Result<bool, String> {
        let guard = COPILOT_CLIENT.lock().await;
        Ok(guard.is_some())
    }

    async fn list_models(
        &self,
        _app_handle: Option<&AppHandle>,
    ) -> Result<Vec<CopilotModelInfo>, String> {
        let client = COPILOT_CLIENT
            .lock()
            .await
            .clone()
            .ok_or("Copilot client not started")?;
        let models = tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, client.list_models())
            .await
            .map_err(|_| "Timed out listing Copilot models".to_string())?
            .map_err(|e| format!("Failed to list models: {}", e))?;

        Ok(models
            .iter()
            .map(|m| CopilotModelInfo {
                id: m.id.clone(),
                name: m.name.clone(),
                description: None,
                context_window: None,
                supports_vision: None,
                supported_reasoning_efforts: if m.capabilities.supports.reasoning_effort {
                    m.supported_reasoning_efforts
                        .clone()
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|id| {
                            let id = id.trim();
                            (!id.is_empty()).then(|| ReasoningEffortOption {
                                id: id.to_string(),
                                name: reasoning_effort_display_name(id),
                                description: None,
                            })
                        })
                        .collect()
                } else {
                    Vec::new()
                },
                default_reasoning_effort: m
                    .capabilities
                    .supports
                    .reasoning_effort
                    .then(|| m.default_reasoning_effort.clone())
                    .flatten(),
                is_default: false,
            })
            .collect())
    }

    async fn get_auth_status(
        &self,
        _app_handle: Option<&AppHandle>,
    ) -> Result<CopilotAuthStatus, String> {
        let client = COPILOT_CLIENT
            .lock()
            .await
            .clone()
            .ok_or("Copilot client not started")?;
        let status = tokio::time::timeout(SDK_CONTROL_RPC_TIMEOUT, client.get_auth_status())
            .await
            .map_err(|_| "Timed out checking Copilot authentication".to_string())?
            .map_err(|e| format!("Failed to get auth status: {}", e))?;

        Ok(CopilotAuthStatus {
            authenticated: status.is_authenticated,
            login: status.login.clone(),
            message: None,
        })
    }
}

#[async_trait]
impl FlowPilotAgentBackend for ExternalCodeAgentBackend {
    fn kind(&self) -> FlowPilotAgentBackendKind {
        self.kind
    }

    async fn start(&self, options: FlowPilotBackendStartOptions) -> Result<(), String> {
        let cli = find_cli_resolution(self.kind, options.app_handle.as_ref()).ok_or_else(|| {
            actionable_external_agent_failure(
                self.kind,
                &external_agent_cli_resolution_failure(self.kind),
            )
        })?;
        let version = probe_external_agent_cli(self.kind, &cli.executable, &cli.path_dirs)
            .await
            .map_err(|error| actionable_external_agent_failure(self.kind, &error))?;
        let auth = probe_external_agent_auth(self.kind, &cli)
            .await
            .map_err(|error| actionable_external_agent_failure(self.kind, &error))?;
        if !auth.authenticated {
            return Err(actionable_external_agent_failure(
                self.kind,
                &format!(
                    "{} authentication failed: {}",
                    self.kind.label(),
                    auth.detail
                ),
            ));
        }
        let mut guard = EXTERNAL_AGENT_BACKENDS.lock().await;
        guard.insert(self.kind);
        tracing::info!(
            backend = self.kind.label(),
            executable = %cli.executable.display(),
            source = ?cli.source,
            version = %version,
            "enabled external FlowPilot backend"
        );
        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        let mut guard = EXTERNAL_AGENT_BACKENDS.lock().await;
        guard.remove(&self.kind);
        Ok(())
    }

    async fn is_running(&self) -> Result<bool, String> {
        let guard = EXTERNAL_AGENT_BACKENDS.lock().await;
        Ok(guard.contains(&self.kind))
    }

    async fn list_models(
        &self,
        app_handle: Option<&AppHandle>,
    ) -> Result<Vec<CopilotModelInfo>, String> {
        let mut models = Vec::new();
        match self.kind {
            FlowPilotAgentBackendKind::Codex => {
                // Codex model availability depends on whether the user is
                // authenticated with a ChatGPT account, API key, enterprise
                // policy, and the installed Codex runtime version, so the options
                // are discovered from Codex itself (its `app-server` `model/list`)
                // rather than hard-coded. "default" is always offered first so the
                // user can defer to Codex's own configured/runtime model.
                let cli = find_cli_resolution(self.kind, app_handle).ok_or_else(|| {
                    actionable_external_agent_failure(
                        self.kind,
                        &external_agent_cli_resolution_failure(self.kind),
                    )
                })?;
                let discovered_models = list_codex_models_via_app_server(&cli)
                    .await
                    .map_err(|error| actionable_external_agent_failure(self.kind, &error))?;
                models = codex_models_with_configured_default(discovered_models);
            }
            FlowPilotAgentBackendKind::ClaudeCode => {
                // The Claude Code CLI exposes no model-listing subcommand, so the
                // options are discovered from its own auth-aware `initialize`
                // handshake (the same list the Agent SDK's `supportedModels()`
                // returns) rather than hard-coded. That catalog already includes
                // a "default (recommended)" entry, so nothing is prepended.
                let cli = find_cli_resolution(self.kind, app_handle).ok_or_else(|| {
                    actionable_external_agent_failure(
                        self.kind,
                        &external_agent_cli_resolution_failure(self.kind),
                    )
                })?;
                let discovered = list_claude_models_via_control_protocol(&cli)
                    .await
                    .map_err(|error| actionable_external_agent_failure(self.kind, &error))?;
                for model in discovered {
                    if !models.iter().any(|existing| existing.id == model.id) {
                        models.push(model);
                    }
                }
                if models.is_empty() {
                    let mut configured_default =
                        CopilotModelInfo::basic("default", "Claude Code configured default");
                    configured_default.is_default = true;
                    models.push(configured_default);
                }
            }
            FlowPilotAgentBackendKind::GithubCopilot => {
                let mut configured_default =
                    CopilotModelInfo::basic("default", "GitHub Copilot configured default");
                configured_default.is_default = true;
                models.push(configured_default);
            }
        }
        Ok(models)
    }

    async fn get_auth_status(
        &self,
        app_handle: Option<&AppHandle>,
    ) -> Result<CopilotAuthStatus, String> {
        let Some(resolution) = find_cli_resolution(self.kind, app_handle) else {
            return Ok(CopilotAuthStatus {
                authenticated: false,
                login: None,
                message: Some(actionable_external_agent_failure(
                    self.kind,
                    &external_agent_cli_resolution_failure(self.kind),
                )),
            });
        };
        let executable = resolution.executable.display().to_string();
        match probe_external_agent_auth(self.kind, &resolution).await {
            Ok(auth) if auth.authenticated => Ok(CopilotAuthStatus {
                authenticated: true,
                login: auth.login,
                message: Some(format!(
                    "{} CLI is ready at {executable} ({:?}). {}",
                    self.kind.label(),
                    resolution.source,
                    auth.detail
                )),
            }),
            Ok(auth) => Ok(CopilotAuthStatus {
                authenticated: false,
                login: auth.login,
                message: Some(actionable_external_agent_failure(
                    self.kind,
                    &format!(
                        "{} authentication failed: {}",
                        self.kind.label(),
                        auth.detail
                    ),
                )),
            }),
            Err(error) => Ok(CopilotAuthStatus {
                authenticated: false,
                login: None,
                message: Some(actionable_external_agent_failure(self.kind, &error)),
            }),
        }
    }

    async fn status(&self, app_handle: Option<&AppHandle>) -> FlowPilotBackendStatus {
        let resolution = find_cli_resolution(self.kind, app_handle);
        let source = resolution.as_ref().map(|resolution| resolution.source);
        let executable = resolution
            .as_ref()
            .map(|resolution| resolution.executable.display().to_string());
        let available = executable.is_some();
        let running = self.is_running().await.unwrap_or(false);
        FlowPilotBackendStatus {
            backend: self.kind,
            label: self.kind.label().to_string(),
            available,
            running,
            executable,
            message: Some(if available {
                format!(
                    "{} uses FlowPilot's shared prompt/tool surface through a session-local MCP bridge ({source:?}).",
                    self.kind.label(),
                )
            } else {
                format!(
                    "{} CLI was not found. Install it or set {}.",
                    self.kind.label(),
                    self.kind.env_path_var()
                )
            }),
            transport: FlowPilotAgentTransportKind::Mcp,
            capabilities: FlowPilotAgentCapabilitySet::for_status(FlowPilotAgentTransportKind::Mcp),
        }
    }
}

pub(super) fn parse_agent_backend(backend: String) -> Result<FlowPilotAgentBackendKind, String> {
    FlowPilotAgentBackendKind::parse(&backend)
        .ok_or_else(|| format!("Unsupported FlowPilot backend: {backend}"))
}

//! External-provider failures and actionable recovery messages.

use super::backend_types::FlowPilotAgentBackendKind;
use super::workflow_state::WorkflowToolLoopSnapshot;

pub(super) const EXTERNAL_AGENT_TOOL_CALL_ID: &str = "external-agent";

/// Result of one external agent CLI run. `error` carries a non-fatal failure (agent error event,
/// non-zero exit) when partial text or a resumable session was produced.
pub(super) struct ExternalAgentRunOutput {
    pub(super) text: String,
    pub(super) error: Option<String>,
    /// Provider session captured from the stream, including phases stopped before their first
    /// answer. Continuations resume this transcript with the host's current recovery state.
    pub(super) session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExternalAgentExitKind {
    UserCancelled,
    TransientInfrastructure,
    Permanent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExternalAgentFailureCategory {
    CliMissing,
    CliNotExecutable,
    LocalPermission,
    Authentication,
    AccountAccess,
    RateLimit,
    Network,
    Model,
    McpConnection,
    Protocol,
    HostWorkflow,
    UserCancelled,
    Input,
    LocalEnvironment,
    Process,
}

pub(super) fn classify_external_agent_failure(
    error: &str,
    cancelled: bool,
) -> ExternalAgentExitKind {
    if cancelled {
        return ExternalAgentExitKind::UserCancelled;
    }
    let normalized = error.to_ascii_lowercase();
    let permanent_markers = [
        "cancelled by user",
        "canceled by user",
        "user aborted",
        "authentication",
        "unauthorized",
        "forbidden",
        "invalid api key",
        "permission denied",
        "billing",
        "unsupported",
        "not installed",
        "executable was not found",
        "invalid request",
        "context length",
        "prompt is too long",
        "request too large",
    ];
    if permanent_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return ExternalAgentExitKind::Permanent;
    }
    let transient_markers = [
        "connection reset",
        "connection refused",
        "connection closed",
        "connection lost",
        "disconnected",
        "broken pipe",
        "unexpected eof",
        "end of stream",
        "stream closed",
        "transport",
        "timed out",
        "timeout",
        "temporarily unavailable",
        "overloaded",
        "rate limit",
        "network error",
        "dns error",
        "http 429",
        "http 502",
        "http 503",
        "http 504",
        "http 529",
        // Host-initiated phase end: the shared session circuit opened and every further mutation
        // tool would be refused, so the phase was cancelled to grant a bounded continuation.
        "zero-progress circuit",
    ];
    if transient_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        ExternalAgentExitKind::TransientInfrastructure
    } else {
        ExternalAgentExitKind::Permanent
    }
}

pub(super) fn classify_external_agent_user_failure(error: &str) -> ExternalAgentFailureCategory {
    let normalized = error.to_ascii_lowercase();

    if [
        "flowpilot external agent run was cancelled",
        "flowpilot run was cancelled",
        "cancelled by user",
        "canceled by user",
        "user cancelled",
        "user canceled",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return ExternalAgentFailureCategory::UserCancelled;
    }

    if [
        "nested_run_wall_clock_budget_exhausted",
        "pre-draft source checkpoint",
        "zero-progress circuit",
        "provider continuation budget",
        "workflow draft needs attention",
        "workflow validation",
        "compiler diagnostics",
        "no board commands were queued",
        "the external agent exhausted its",
        "without queueing changes",
        "retained the most complete flowscript draft",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return ExternalAgentFailureCategory::HostWorkflow;
    }

    if [
        "prompt image",
        "failed to decode prompt image",
        "unsupported prompt image",
        "invalid image attachment",
        "context length",
        "context_length",
        "prompt is too long",
        "request too large",
        "maximum context",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return ExternalAgentFailureCategory::Input;
    }

    if [
        "failed to create attachment directory",
        "failed to write attachment",
        "failed to write claude mcp config",
        "failed to serialize claude mcp config",
        "no space left on device",
        "temporary directory",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return ExternalAgentFailureCategory::LocalEnvironment;
    }

    let local_execution_context = [
        "failed to start",
        "failed to run",
        "cli at ",
        "executable",
        "spawn",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    let local_permission = [
        "permission denied",
        "operation not permitted",
        "access is denied",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));

    if [
        "not executable",
        "os error 13",
        "bad cpu type",
        "exec format error",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
        || (local_execution_context && local_permission)
    {
        return ExternalAgentFailureCategory::CliNotExecutable;
    }

    let explicit_cli_resolution_failure = [
        "cli was not found",
        "executable was not found",
        "executable does not exist",
        "does not contain an executable",
        "executable was not found on",
        "cli cannot be resolved",
        "codex_cli_path",
        "claude_code_cli_path",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    let missing_cli_file = [
        "no such file or directory",
        "cannot find the file",
        "could not find executable",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
        && local_execution_context;
    if explicit_cli_resolution_failure || missing_cli_file {
        return ExternalAgentFailureCategory::CliMissing;
    }

    let local_data_context = [
        "configuration",
        "config file",
        "settings file",
        "credentials file",
        "credential store",
        "keychain",
        "filesystem",
        "failed to read",
        "failed to open",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    if local_permission && local_data_context {
        return ExternalAgentFailureCategory::LocalPermission;
    }

    if normalized.contains("cli probe timed out")
        || normalized.contains("--version exited")
        || normalized.contains("authentication status check timed out")
    {
        return ExternalAgentFailureCategory::Process;
    }

    let protocol_failure = [
        "unknown option",
        "unexpected argument",
        "unrecognized option",
        "unknown subcommand",
        "unsupported command",
        "protocol",
        "app-server closed",
        "control session closed",
        "before returning models",
        "initialize failed",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    let explicit_auth_or_access_failure = [
        "authentication failed",
        "failed to authenticate",
        "not authenticated",
        "not logged in",
        "login expired",
        "invalid api key",
        "invalid authentication credentials",
        "oauth",
        "token expired",
        "token revoked",
        "token_invalidated",
        "unauthorized",
        "401",
        "forbidden",
        "403",
        "organization",
        "billing",
        "subscription",
        "entitlement",
    ]
    .iter()
    .any(|marker| normalized.contains(marker));
    if protocol_failure && !explicit_auth_or_access_failure {
        return ExternalAgentFailureCategory::Protocol;
    }

    if [
        "rate limit",
        "rate_limit",
        "too many requests",
        "http 429",
        "status 429",
        "429:",
        "429)",
        "code 429",
        "overloaded",
        "http 529",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return ExternalAgentFailureCategory::RateLimit;
    }

    if [
        "forbidden",
        "http 403",
        "status 403",
        "403:",
        "403)",
        "code 403",
        "http 402",
        "status 402",
        "402:",
        "402)",
        "code 402",
        "payment required",
        "organization has been disabled",
        "organization is disabled",
        "org_not_allowed",
        "oauth_org_not_allowed",
        "billing",
        "insufficient quota",
        "insufficient_quota",
        "credit balance",
        "subscription access",
        "subscription required",
        "eligible plan",
        "not available on your plan",
        "entitlement",
        "does not have access",
        "access has been disabled",
        "permission denied by policy",
        "account access denied",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return ExternalAgentFailureCategory::AccountAccess;
    }

    if [
        "authentication failed",
        "failed to authenticate",
        "unauthorized",
        "not authenticated",
        "not logged in",
        "login required",
        "login expired",
        "sign in required",
        "invalid api key",
        "invalid_api_key",
        "authentication_failed",
        "oauth session expired",
        "oauth token",
        "token expired",
        "token revoked",
        "token_invalidated",
        "invalid authentication credentials",
        "http 401",
        "status 401",
        "401 unauthorized",
        "401:",
        "401)",
        "code 401",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return ExternalAgentFailureCategory::Authentication;
    }

    if [
        "mcp server connection failed",
        "mcp connection",
        "mcp server",
        "flowpilot tools are unavailable",
        "flowpilot mcp",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return ExternalAgentFailureCategory::McpConnection;
    }

    if [
        "connection reset",
        "connection refused",
        "connection closed",
        "connection lost",
        "unable to connect",
        "could not connect",
        "network error",
        "network unavailable",
        "dns error",
        "name resolution",
        "proxy",
        "certificate",
        "tls",
        "timed out",
        "timeout",
        "temporarily unavailable",
        "http 502",
        "http 503",
        "http 504",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
    {
        return ExternalAgentFailureCategory::Network;
    }

    let contextual_model_failure = normalized.contains("model")
        && [
            "does not exist",
            "could not find",
            "do not have access",
            "don't have access",
        ]
        .iter()
        .any(|marker| normalized.contains(marker));
    if contextual_model_failure
        || [
            "model not found",
            "model is not available",
            "model unavailable",
            "unsupported model",
            "unknown model",
            "invalid model",
            "selected model",
        ]
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return ExternalAgentFailureCategory::Model;
    }

    ExternalAgentFailureCategory::Process
}

fn external_agent_login_command(kind: FlowPilotAgentBackendKind) -> &'static str {
    match kind {
        FlowPilotAgentBackendKind::Codex => "codex login",
        FlowPilotAgentBackendKind::ClaudeCode => "claude auth login",
        FlowPilotAgentBackendKind::GithubCopilot => "copilot login",
    }
}

fn external_agent_auth_status_command(kind: FlowPilotAgentBackendKind) -> &'static str {
    match kind {
        FlowPilotAgentBackendKind::Codex => "codex login status",
        FlowPilotAgentBackendKind::ClaudeCode => "claude auth status --text",
        FlowPilotAgentBackendKind::GithubCopilot => "copilot status",
    }
}

pub(super) fn actionable_external_agent_failure(
    kind: FlowPilotAgentBackendKind,
    error: &str,
) -> String {
    let label = kind.label();
    let cli_name = kind.cli_name();
    let category = classify_external_agent_user_failure(error);
    let (title, action) = match category {
        ExternalAgentFailureCategory::CliMissing => (
            format!("{label} CLI was not found."),
            format!(
                "Install {label}, fully quit and reopen Flow-Like, then verify `{cli_name} --version`. If it is installed in a custom location, set {} to the full executable path.",
                kind.env_path_var()
            ),
        ),
        ExternalAgentFailureCategory::CliNotExecutable => (
            format!("Flow-Like cannot run the {label} CLI."),
            format!(
                "Verify `{cli_name} --version` in a terminal. Reinstall the CLI or fix the executable selected by {}; then fully quit and reopen Flow-Like.",
                kind.env_path_var()
            ),
        ),
        ExternalAgentFailureCategory::LocalPermission => (
            format!("{label} cannot access its local configuration or credentials."),
            format!(
                "Check file and keychain permissions for the signed-in user, then verify `{}`. Reinstall the CLI only if its own status command still fails.",
                external_agent_auth_status_command(kind)
            ),
        ),
        ExternalAgentFailureCategory::Authentication => (
            format!("{label} needs you to sign in again."),
            if [
                "invalid api key",
                "invalid_api_key",
                "missing api key",
                "anthropic_api_key",
                "anthropic_auth_token",
                "openai_api_key",
            ]
            .iter()
            .any(|marker| error.to_ascii_lowercase().contains(marker))
            {
                match kind {
                    FlowPilotAgentBackendKind::ClaudeCode => format!(
                        "Update or unset the invalid `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` environment credential, fully quit and reopen Flow-Like, then verify with `{}`. Run `{}` if you want to switch back to subscription sign-in.",
                        external_agent_auth_status_command(kind),
                        external_agent_login_command(kind)
                    ),
                    FlowPilotAgentBackendKind::Codex => format!(
                        "Store a valid API key again with `codex login --with-api-key`, or switch to account sign-in with `{}`. Verify with `{}`, then retry in Flow-Like.",
                        external_agent_login_command(kind),
                        external_agent_auth_status_command(kind)
                    ),
                    FlowPilotAgentBackendKind::GithubCopilot => format!(
                        "Replace the invalid API credential, run `{}`, and verify with `{}` before retrying.",
                        external_agent_login_command(kind),
                        external_agent_auth_status_command(kind)
                    ),
                }
            } else {
                format!(
                    "Run `{}`, complete sign-in, verify with `{}`, then retry in Flow-Like.",
                    external_agent_login_command(kind),
                    external_agent_auth_status_command(kind)
                )
            },
        ),
        ExternalAgentFailureCategory::AccountAccess => (
            format!("{label} account access was denied."),
            format!(
                "Check that the signed-in account has an eligible plan, billing, model access, and organization permission. Verify the active account with `{}`; sign in with a different account if needed.",
                external_agent_auth_status_command(kind)
            ),
        ),
        ExternalAgentFailureCategory::RateLimit => (
            format!("{label} is temporarily rate-limited."),
            "Wait a moment and retry. If it continues, check the provider's usage limits or service status."
                .to_string(),
        ),
        ExternalAgentFailureCategory::Network => (
            format!("{label} could not reach its service."),
            format!(
                "Check your internet connection, VPN, proxy, firewall, and TLS certificate settings. Confirm `{}` completes in the same environment, then retry.",
                external_agent_auth_status_command(kind)
            ),
        ),
        ExternalAgentFailureCategory::Model => (
            format!("The selected {label} model is unavailable."),
            "Refresh the model list and choose an available model. If the catalog still fails, update the CLI and verify your account's model access."
                .to_string(),
        ),
        ExternalAgentFailureCategory::McpConnection => (
            format!("{label} could not connect to FlowPilot tools."),
            "Retry once. If it repeats, fully quit and reopen Flow-Like and make sure local security software is not blocking loopback connections."
                .to_string(),
        ),
        ExternalAgentFailureCategory::Protocol => (
            format!("The installed {label} CLI is not compatible with FlowPilot."),
            format!(
                "Update or reinstall {label}, verify `{cli_name} --version`, fully quit and reopen Flow-Like, then retry."
            ),
        ),
        ExternalAgentFailureCategory::HostWorkflow => (
            format!("{label} stopped before completing the requested workflow."),
            "Review the retained FlowScript or compiler diagnostics, then continue or retry the workflow. The CLI installation and sign-in do not need to be changed for this host-side limit."
                .to_string(),
        ),
        ExternalAgentFailureCategory::UserCancelled => (
            "FlowPilot run was cancelled.".to_string(),
            "Start a new request when you are ready to continue.".to_string(),
        ),
        ExternalAgentFailureCategory::Input => {
            if [
                "context length",
                "context_length",
                "prompt is too long",
                "request too large",
                "maximum context",
            ]
            .iter()
            .any(|marker| error.to_ascii_lowercase().contains(marker))
            {
                (
                    format!("The request is too large for {label}."),
                    "Start a new conversation or shorten the prompt/history. Remove large attachments or unnecessary context, then retry."
                        .to_string(),
                )
            } else {
                (
                    format!("{label} could not use an attached image."),
                    "Remove and re-attach the image in PNG, JPEG, GIF, or WebP format. Compress or resize it if it exceeds 64 MB, then retry."
                        .to_string(),
                )
            }
        }
        ExternalAgentFailureCategory::LocalEnvironment => (
            "Flow-Like could not prepare the local agent session.".to_string(),
            "Check available disk space and permissions for the system temporary directory, then fully quit and reopen Flow-Like before retrying."
                .to_string(),
        ),
        ExternalAgentFailureCategory::Process => (
            format!("{label} stopped unexpectedly."),
            format!(
                "Run `{cli_name} --version` and `{}` in a terminal. Update or reinstall the CLI if either command fails, then retry.",
                external_agent_auth_status_command(kind)
            ),
        ),
    };
    let detail = flow_like::flow::copilot::stream::safe_text_preview(error.trim(), 1_200);
    if detail.is_empty() {
        format!("{title}\n\nHow to fix: {action}")
    } else {
        format!("{title}\n\nHow to fix: {action}\n\nTechnical details: {detail}")
    }
}

pub(super) fn external_agent_run_failure(
    result: &Result<ExternalAgentRunOutput, String>,
) -> Option<&str> {
    match result {
        Ok(output) => output.error.as_deref(),
        Err(error) => Some(error.as_str()),
    }
}

pub(super) fn can_resume_external_workflow_after_failure(
    snapshot: Option<&WorkflowToolLoopSnapshot>,
    error: &str,
    cancelled: bool,
) -> bool {
    let Some(snapshot) = snapshot else {
        return false;
    };
    !snapshot.queued
        && classify_external_agent_failure(error, cancelled)
            == ExternalAgentExitKind::TransientInfrastructure
}

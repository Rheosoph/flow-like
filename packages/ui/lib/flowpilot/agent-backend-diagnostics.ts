import type { AIProvider } from "../../components/flowpilot/types";

export type AgentBackendDiagnosticKind =
	| "cli-missing"
	| "cli-path"
	| "cli-not-executable"
	| "local-permission"
	| "auth"
	| "policy-billing"
	| "rate-limit"
	| "network-timeout"
	| "model-version-protocol"
	| "mcp"
	| "workflow"
	| "input"
	| "local-environment"
	| "generic";

export interface AgentBackendDiagnostic {
	kind: AgentBackendDiagnosticKind;
	title: string;
	message: string;
	command?: string;
	rawError: string;
	retryable: boolean;
}

interface ProviderDiagnosticGuide {
	label: string;
	executable?: string;
	pathVariable?: string;
	verifyCommand?: string;
	loginCommand?: string;
	authStatusCommand?: string;
}

const PROVIDER_GUIDES: Record<string, ProviderDiagnosticGuide> = {
	"github-copilot": {
		label: "GitHub Copilot",
		executable: "copilot",
		pathVariable: "COPILOT_CLI_PATH",
		verifyCommand: "copilot --version",
		loginCommand: "copilot login",
		authStatusCommand: "copilot --version",
	},
	codex: {
		label: "Codex",
		executable: "codex",
		pathVariable: "CODEX_CLI_PATH",
		verifyCommand: "codex --version",
		loginCommand: "codex login",
		authStatusCommand: "codex login status",
	},
	"claude-code": {
		label: "Claude Code",
		executable: "claude",
		pathVariable: "CLAUDE_CODE_CLI_PATH",
		verifyCommand: "claude --version && claude doctor",
		loginCommand: "claude auth login",
		authStatusCommand: "claude auth status --text",
	},
	bits: {
		label: "FlowPilot",
	},
};

function guideFor(provider: AIProvider | string): ProviderDiagnosticGuide {
	const normalized = provider === "copilot" ? "github-copilot" : provider;
	return (
		PROVIDER_GUIDES[normalized] ?? {
			label: normalized || "Agent backend",
		}
	);
}

function messageFromUnknown(error: unknown): string {
	if (typeof error === "string") return error.trim();
	if (error instanceof Error) return error.message.trim();
	if (error && typeof error === "object") {
		const candidate = error as Record<string, unknown>;
		for (const key of ["message", "error", "details", "reason"]) {
			const value = candidate[key];
			if (typeof value === "string" && value.trim()) return value.trim();
			if (value instanceof Error && value.message.trim()) {
				return value.message.trim();
			}
		}
		try {
			return JSON.stringify(error);
		} catch {
			return String(error);
		}
	}
	return error == null ? "" : String(error).trim();
}

function redactAndBound(raw: string): string {
	return raw
		.replace(/\bBearer\s+[^\s,;]+/gi, "Bearer [redacted]")
		.replace(
			/\b(api[_ -]?key|access[_ -]?token|refresh[_ -]?token)\b(\s*[:=]\s*)[^\s,;]+/gi,
			"$1$2[redacted]",
		)
		.replace(/\bsk-[a-z0-9_-]{12,}\b/gi, "[redacted API key]")
		.slice(0, 2_000);
}

function containsAny(value: string, markers: readonly string[]) {
	return markers.some((marker) => value.includes(marker));
}

export function isAgentBackendCancellation(error: unknown): boolean {
	const normalized = messageFromUnknown(error).toLowerCase();
	return containsAny(normalized, [
		"flowpilot external agent run was cancelled",
		"flowpilot run was cancelled",
		"cancelled by user",
		"canceled by user",
		"user cancelled",
		"user canceled",
	]);
}

function diagnostic(
	kind: AgentBackendDiagnosticKind,
	title: string,
	message: string,
	command: string | undefined,
	rawError: string,
): AgentBackendDiagnostic {
	return {
		kind,
		title,
		message,
		command,
		rawError,
		retryable: true,
	};
}

/**
 * Turn backend and transport errors into stable, user-facing recovery guidance.
 *
 * Matching is deliberately based on broad CLI/HTTP/runtime phrases because Tauri
 * errors arrive as strings from several independent provider processes. Keep the
 * raw error on the diagnostic for logs, but show the actionable fields in UI.
 */
export function classifyAgentBackendError(
	provider: AIProvider | string,
	error: unknown,
): AgentBackendDiagnostic | null {
	const rawError = redactAndBound(messageFromUnknown(error));
	if (!rawError) return null;

	const normalized = rawError.toLowerCase();
	if (isAgentBackendCancellation(rawError)) return null;

	const guide = guideFor(provider);
	const usesExternalCli = Boolean(guide.executable);
	const cliContext = containsAny(normalized, [
		"failed to start",
		"failed to run",
		"cli at ",
		"executable",
		"spawn",
		"--version",
	]);
	const configuredPath =
		Boolean(
			guide.pathVariable &&
				normalized.includes(guide.pathVariable.toLowerCase()),
		) ||
		containsAny(normalized, [
			"configured cli path",
			"configured executable path",
			"invalid executable path",
			"cli path points",
			"executable path points",
		]);

	if (
		containsAny(normalized, [
			"context length",
			"context_length",
			"prompt is too long",
			"request too large",
			"maximum context",
		])
	) {
		return diagnostic(
			"input",
			`The request is too large for ${guide.label}`,
			"Start a new conversation or shorten the prompt/history. Remove large attachments or unnecessary context, then retry.",
			undefined,
			rawError,
		);
	}

	if (
		containsAny(normalized, [
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
			"stopped before completing the requested workflow",
		])
	) {
		return diagnostic(
			"workflow",
			"FlowPilot workflow stopped before completion",
			`${rawError} Review any retained FlowScript or compiler diagnostics, then continue or retry the workflow. The provider CLI and sign-in do not need to be changed.`,
			undefined,
			rawError,
		);
	}

	if (
		containsAny(normalized, [
			"prompt image",
			"failed to decode prompt image",
			"unsupported prompt image",
			"invalid image attachment",
			"could not use an attached image",
		])
	) {
		return diagnostic(
			"input",
			`${guide.label} could not use an attached image`,
			"Remove and re-attach the image in PNG, JPEG, GIF, or WebP format. Compress or resize it if it exceeds 64 MB, then retry.",
			undefined,
			rawError,
		);
	}

	if (
		containsAny(normalized, [
			"failed to create attachment directory",
			"failed to write attachment",
			"failed to write claude mcp config",
			"failed to serialize claude mcp config",
			"no space left on device",
			"temporary directory",
			"could not prepare the local agent session",
		])
	) {
		return diagnostic(
			"local-environment",
			"Flow-Like could not prepare the local agent session",
			"Check available disk space and permissions for the system temporary directory, then fully quit and reopen Flow-Like before retrying.",
			undefined,
			rawError,
		);
	}

	if (
		usesExternalCli &&
		containsAny(normalized, [
			"permission denied",
			"operation not permitted",
			"access is denied",
		]) &&
		containsAny(normalized, [
			"configuration",
			"config file",
			"settings file",
			"credentials file",
			"credential store",
			"keychain",
			"filesystem",
			"failed to read",
			"failed to open",
			"local configuration or credentials",
		])
	) {
		return diagnostic(
			"local-permission",
			`${guide.label} cannot access local credentials`,
			`Check file and keychain permissions for the signed-in user, then verify ${guide.authStatusCommand ?? "the provider CLI"}. Reinstall the CLI only if its own status command still fails.`,
			guide.authStatusCommand ?? guide.verifyCommand,
			rawError,
		);
	}

	if (
		usesExternalCli &&
		(containsAny(normalized, [
			"not executable",
			"exec format error",
			"permission denied while spawning",
			"permission denied launching",
			"cannot execute binary",
			"is a directory",
			"eacces",
		]) ||
			(cliContext &&
				normalized.includes("permission denied") &&
				!normalized.includes("request")))
	) {
		return diagnostic(
			"cli-not-executable",
			`${guide.label} CLI cannot run`,
			`Point ${guide.pathVariable ?? "the CLI path setting"} at the actual executable and make sure Flow-Like can execute it. Do not use a shell alias.`,
			guide.verifyCommand,
			rawError,
		);
	}

	if (
		usesExternalCli &&
		configuredPath &&
		!containsAny(normalized, ["cli was not found", "cli not found"]) &&
		containsAny(normalized, [
			"does not exist",
			"not found",
			"no such file",
			"invalid",
			"points to",
			"directory",
		])
	) {
		return diagnostic(
			"cli-path",
			`${guide.label} CLI path is invalid`,
			`Update ${guide.pathVariable ?? "the CLI path setting"} to the full executable path, then fully quit and reopen Flow-Like.`,
			guide.verifyCommand,
			rawError,
		);
	}

	if (
		usesExternalCli &&
		(containsAny(normalized, [
			"cli was not found",
			"cli not found",
			"command not found",
			"executable was not found",
			"executable not found",
			"could not find executable",
			"cannot find executable",
			"program not found",
		]) ||
			(normalized.includes("enoent") && cliContext) ||
			(normalized.includes("no such file or directory") && cliContext))
	) {
		return diagnostic(
			"cli-missing",
			`${guide.label} CLI not found`,
			`Install the ${guide.label} CLI, verify it in a new terminal, then fully quit and reopen Flow-Like. If it is installed in a custom location, set ${guide.pathVariable ?? "the provider CLI path"} to the full executable path.`,
			guide.verifyCommand,
			rawError,
		);
	}

	if (
		/\b401\b/.test(normalized) ||
		containsAny(normalized, [
			"unauthorized",
			"unauthenticated",
			"authentication required",
			"authentication failed",
			"not authenticated",
			"login required",
			"log in required",
			"not logged in",
			"not signed in",
			"please log in",
			"please login",
			"signed out",
			"session expired",
			"token expired",
			"expired token",
			"invalid token",
			"invalid api key",
			"missing api key",
			"oauth token",
			"refresh token",
			"credentials have expired",
			"failed to authenticate",
			"could not authenticate",
			"token revoked",
			"revoked token",
			"token invalidated",
			"run /login",
		])
	) {
		const apiCredentialFailure = containsAny(normalized, [
			"invalid api key",
			"invalid_api_key",
			"missing api key",
			"anthropic_api_key",
			"anthropic_auth_token",
			"openai_api_key",
		]);
		const message = apiCredentialFailure
			? guide.label === "Claude Code"
				? `Update or unset the invalid ANTHROPIC_API_KEY / ANTHROPIC_AUTH_TOKEN environment credential, fully quit and reopen Flow-Like, then verify with ${guide.authStatusCommand}. Run ${guide.loginCommand} if you want to switch back to subscription sign-in.`
				: guide.label === "Codex"
					? `Store a valid API key again with codex login --with-api-key, or switch to account sign-in with ${guide.loginCommand}. Verify with ${guide.authStatusCommand}, then retry.`
					: "Replace the invalid API credential, sign in again, and retry."
			: usesExternalCli
				? `Sign in again, complete any browser prompt, then verify the session with ${guide.authStatusCommand ?? "the provider CLI"} before retrying.`
				: "Sign in again, complete any browser prompt, then retry.";
		return diagnostic(
			"auth",
			`${guide.label} sign-in required`,
			message,
			guide.loginCommand ?? guide.authStatusCommand,
			rawError,
		);
	}

	if (
		/\b429\b/.test(normalized) ||
		containsAny(normalized, [
			"rate limit",
			"too many requests",
			"resource exhausted",
			"retry-after",
			"request limit reached",
			"capacity exceeded",
			"temporarily overloaded",
			"server overloaded",
		])
	) {
		return diagnostic(
			"rate-limit",
			`${guide.label} rate limit reached`,
			"Wait for the provider limit to reset, then retry. If this continues, check the account's usage limits.",
			guide.authStatusCommand ?? guide.verifyCommand,
			rawError,
		);
	}

	if (
		/\b40[23]\b/.test(normalized) ||
		containsAny(normalized, [
			"forbidden",
			"billing",
			"payment required",
			"insufficient_quota",
			"insufficient quota",
			"credit balance",
			"quota exceeded",
			"usage quota",
			"subscription",
			"plan does not include",
			"plan doesn't include",
			"organization policy",
			"workspace policy",
			"enterprise policy",
			"disabled by your admin",
			"access denied",
			"not entitled",
			"permission denied",
			"permission denied by policy",
		])
	) {
		return diagnostic(
			"policy-billing",
			`${guide.label} access is blocked`,
			"Check the provider subscription, billing balance, and organization policy for this account. An administrator may need to enable CLI or model access.",
			guide.authStatusCommand ?? guide.verifyCommand,
			rawError,
		);
	}

	if (
		/\bmcp\b/.test(normalized) ||
		containsAny(normalized, [
			"model context protocol",
			"tool server failed",
			"tool server disconnected",
			"failed to connect tool server",
			"tool handshake failed",
		])
	) {
		return diagnostic(
			"mcp",
			`${guide.label} tool connection failed`,
			"Restart the provider connection to rebuild Flow-Like's temporary MCP bridge. If it still fails, fully quit and reopen Flow-Like.",
			guide.authStatusCommand ?? guide.verifyCommand,
			rawError,
		);
	}

	if (
		/\b(?:408|502|503|504|529)\b/.test(normalized) ||
		containsAny(normalized, [
			"timed out",
			"timeout",
			"network error",
			"network is unreachable",
			"offline",
			"dns error",
			"enotfound",
			"econnreset",
			"econnrefused",
			"econnaborted",
			"connection reset",
			"connection refused",
			"connection closed",
			"connection lost",
			"disconnected",
			"broken pipe",
			"unexpected eof",
			"end of stream",
			"socket closed",
			"tls error",
			"certificate error",
			"proxy error",
			"temporarily unavailable",
			"transport error",
		])
	) {
		return diagnostic(
			"network-timeout",
			`${guide.label} could not reach the provider`,
			"Check the internet connection, VPN, proxy, and firewall, then retry. A provider outage can also cause this error.",
			guide.authStatusCommand ?? guide.verifyCommand,
			rawError,
		);
	}

	if (
		containsAny(normalized, [
			"model not found",
			"unknown model",
			"unsupported model",
			"model is unavailable",
			"model unavailable",
			"invalid model",
			"model/list",
			"closed before returning models",
			"control session closed",
			"initialize failed",
			"version mismatch",
			"incompatible version",
			"unsupported version",
			"minimum version",
			"outdated cli",
			"--version exited",
			"upgrade required",
			"protocol error",
			"protocol mismatch",
			"unsupported protocol",
			"json-rpc",
			"initialize handshake",
			"handshake failed",
			"malformed response",
			"unsupported method",
			"unsupported flowpilot backend",
		])
	) {
		return diagnostic(
			"model-version-protocol",
			usesExternalCli
				? `${guide.label} CLI or model is incompatible`
				: `${guide.label} model is incompatible`,
			usesExternalCli
				? "Update the provider CLI, reconnect it, and choose a model supported by the refreshed catalog. Selecting the configured default can bypass a stale model choice."
				: "Refresh the available models and choose a supported model. Selecting the configured default can bypass a stale model choice.",
			guide.verifyCommand,
			rawError,
		);
	}

	return diagnostic(
		"generic",
		`${guide.label} failed`,
		usesExternalCli
			? `${rawError} Verify the provider CLI and account in a terminal, then retry. If the problem continues, fully quit and reopen Flow-Like.`
			: `${rawError} Retry the request. If the problem continues, fully quit and reopen Flow-Like.`,
		guide.verifyCommand ?? guide.authStatusCommand,
		rawError,
	);
}

export function formatAgentBackendDiagnostic(
	diagnosticValue: AgentBackendDiagnostic,
): string {
	const command = diagnosticValue.command
		? ` Try in a terminal: ${diagnosticValue.command}`
		: "";
	return `${diagnosticValue.title}: ${diagnosticValue.message}${command}`;
}

export function shouldPersistAgentBackendDiagnostic(
	diagnosticValue: AgentBackendDiagnostic,
): boolean {
	return (
		diagnosticValue.kind !== "workflow" && diagnosticValue.kind !== "input"
	);
}

export function formatAgentBackendFailure(
	provider: AIProvider | string,
	error: unknown,
): string {
	if (isAgentBackendCancellation(error)) {
		return messageFromUnknown(error) || "FlowPilot run was cancelled.";
	}
	const diagnosticValue = classifyAgentBackendError(provider, error);
	return diagnosticValue
		? formatAgentBackendDiagnostic(diagnosticValue)
		: `${guideFor(provider).label} failed.`;
}

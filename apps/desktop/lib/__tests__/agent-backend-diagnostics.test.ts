import {
	classifyAgentBackendError,
	formatAgentBackendDiagnostic,
	formatAgentBackendFailure,
	shouldPersistAgentBackendDiagnostic,
} from "@flow-like/flow-like-ui/lib/flowpilot/agent-backend-diagnostics";
import { describe, expect, test } from "vitest";

type TestedProvider = "codex" | "claude-code";

function requireDiagnostic(provider: TestedProvider, error: unknown) {
	const diagnostic = classifyAgentBackendError(provider, error);
	expect(diagnostic).not.toBeNull();
	if (!diagnostic) {
		throw new Error(`Expected a diagnostic for ${provider}`);
	}
	return diagnostic;
}

describe("agent backend error diagnostics", () => {
	test.each([
		{
			kind: "cli-missing",
			provider: "codex",
			error:
				"Codex CLI was not found. Install it or set CODEX_CLI_PATH to the executable path.",
			command: "codex --version",
			guidance: /install/i,
		},
		{
			kind: "cli-path",
			provider: "codex",
			error: "CODEX_CLI_PATH points to /old/bin/codex, which does not exist",
			command: "codex --version",
			guidance: /full executable path/i,
		},
		{
			kind: "cli-not-executable",
			provider: "claude-code",
			error: "Failed to spawn claude: EACCES, permission denied",
			command: "claude --version && claude doctor",
			guidance: /executable/i,
		},
		{
			kind: "local-permission",
			provider: "claude-code",
			error: "Failed to read credentials file: permission denied",
			command: "claude auth status --text",
			guidance: /file and keychain permissions/i,
		},
		{
			kind: "input",
			provider: "codex",
			error: "context length exceeded: prompt is too long",
			command: undefined,
			guidance: /new conversation.*shorten/i,
		},
		{
			kind: "local-environment",
			provider: "claude-code",
			error: "Failed to write Claude MCP config: No space left on device",
			command: undefined,
			guidance: /disk space.*temporary directory/i,
		},
		{
			kind: "auth",
			provider: "codex",
			error: "HTTP 401 Unauthorized: OAuth token expired",
			command: "codex login",
			guidance: /sign in again/i,
		},
		{
			kind: "policy-billing",
			provider: "claude-code",
			error: "HTTP 403: organization policy blocked access to this plan",
			command: "claude auth status --text",
			guidance: /subscription|billing|organization policy/i,
		},
		{
			kind: "rate-limit",
			provider: "codex",
			error: "HTTP 429 Too Many Requests: rate limit reached",
			command: "codex login status",
			guidance: /wait.*retry/i,
		},
		{
			kind: "network-timeout",
			provider: "claude-code",
			error: "Network error: connection timed out behind proxy",
			command: "claude auth status --text",
			guidance: /internet connection.*vpn.*proxy/i,
		},
		{
			kind: "model-version-protocol",
			provider: "codex",
			error: "Protocol mismatch: unsupported model selected by model/list",
			command: "codex --version",
			guidance: /update.*cli.*supported/i,
		},
		{
			kind: "mcp",
			provider: "claude-code",
			error: "MCP server connection failed during tool handshake",
			command: "claude auth status --text",
			guidance: /restart.*connection/i,
		},
		{
			kind: "generic",
			provider: "codex",
			error: new Error("Provider subprocess exited unexpectedly with code 17"),
			command: "codex --version",
			guidance: /verify.*terminal.*retry/i,
		},
	] as const)(
		"classifies $kind failures and supplies actionable recovery guidance",
		({ kind, provider, error, command, guidance }) => {
			const diagnostic = requireDiagnostic(provider, error);
			const rawError = error instanceof Error ? error.message : error;

			expect(diagnostic).toMatchObject({
				kind,
				command,
				rawError,
			});
			expect(diagnostic.title.trim().length).toBeGreaterThan(0);
			expect(diagnostic.message).toMatch(guidance);
			expect(diagnostic.retryable).toBeTypeOf("boolean");
		},
	);

	test.each([
		{
			provider: "codex",
			label: "Codex",
			verifyCommand: "codex --version",
			loginCommand: "codex login",
			statusCommand: "codex login status",
		},
		{
			provider: "claude-code",
			label: "Claude Code",
			verifyCommand: "claude --version",
			loginCommand: "claude auth login",
			statusCommand: "claude auth status --text",
		},
	] as const)(
		"uses $label verification and login commands",
		({ provider, verifyCommand, loginCommand, statusCommand }) => {
			const missing = requireDiagnostic(
				provider,
				`${provider} CLI was not found`,
			);
			const auth = requireDiagnostic(
				provider,
				"HTTP 401 Unauthorized: session expired",
			);

			expect(missing.kind).toBe("cli-missing");
			expect(missing.command).toContain(verifyCommand);
			expect(auth.kind).toBe("auth");
			expect(auth.command).toBe(loginCommand);
			expect(auth.message).toContain(statusCommand);
		},
	);

	test("formats the diagnosis, recovery guidance, and command for display", () => {
		const rawError = "HTTP 401 Unauthorized: token expired";
		const diagnostic = requireDiagnostic("claude-code", rawError);
		const formatted = formatAgentBackendDiagnostic(diagnostic);

		expect(formatted).toContain(diagnostic.title);
		expect(formatted).toContain(diagnostic.message);
		expect(formatted).toContain("claude auth login");
		expect(formatAgentBackendFailure("claude-code", rawError)).toBe(formatted);
	});

	test("distinguishes invalid environment API keys from expired login sessions", () => {
		const claude = requireDiagnostic(
			"claude-code",
			"Invalid API key from ANTHROPIC_API_KEY",
		);
		const codex = requireDiagnostic(
			"codex",
			"Invalid API key from OPENAI_API_KEY",
		);

		expect(claude.message).toContain("ANTHROPIC_API_KEY");
		expect(claude.message).toContain("unset");
		expect(codex.message).toContain("codex login --with-api-key");
	});

	test("keeps an unknown failure useful and preserves the technical detail", () => {
		const rawError = "Provider subprocess exited unexpectedly with code 17";
		const formatted = formatAgentBackendFailure("codex", rawError);

		expect(formatted).toContain("Codex");
		expect(formatted).toContain(rawError);
		expect(formatted).toContain("codex --version");
	});

	test("keeps host workflow limits separate from provider readiness", () => {
		const rawError =
			"NESTED_RUN_WALL_CLOCK_BUDGET_EXHAUSTED: compiler diagnostics retained";
		const diagnostic = requireDiagnostic("codex", rawError);

		expect(diagnostic.kind).toBe("workflow");
		expect(diagnostic.command).toBeUndefined();
		expect(diagnostic.message).toContain(
			"provider CLI and sign-in do not need to be changed",
		);
		expect(shouldPersistAgentBackendDiagnostic(diagnostic)).toBe(false);
	});

	test("does not turn user cancellation into a backend failure", () => {
		const rawError = "FlowPilot external agent run was cancelled";

		expect(classifyAgentBackendError("claude-code", rawError)).toBeNull();
		expect(formatAgentBackendFailure("claude-code", rawError)).toBe(rawError);
	});

	test("returns no diagnosis for an empty failure", () => {
		expect(classifyAgentBackendError("codex", "   ")).toBeNull();
		expect(formatAgentBackendFailure("codex", "   ")).toBe("Codex failed.");
	});
});

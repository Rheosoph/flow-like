export type ITelemetryLlmOperation = "chat" | "embed" | "tool";

export type ITelemetryLlmStatus = "ok" | "error";

export interface ITelemetryLlmTotals {
	calls: number;
	errors: number;
	errorRate: number;
	totalTokens: number;
	promptTokens: number;
	completionTokens: number;
	avgDurationMs: number;
	p95DurationMs: number;
}

export interface ITelemetryLlmModelRow {
	provider: string;
	model: string;
	calls: number;
	errors: number;
	errorRate: number;
	avgDurationMs: number;
	p95DurationMs: number;
	totalTokens: number;
}

export interface ITelemetryLlmProviderRow {
	provider: string;
	model?: string | null;
	calls: number;
	errors: number;
	errorRate: number;
	avgDurationMs: number;
	p95DurationMs: number;
	totalTokens: number;
}

export interface ITelemetryLlmOperationRow {
	operation: string;
	calls: number;
	errorRate: number;
}

export interface ITelemetryLlmErrorRow {
	errorKind: string;
	count: number;
}

export interface ITelemetryLlmTrendPoint {
	ts: string;
	calls: number;
	errors: number;
	p95DurationMs: number;
}

export interface ITelemetryLlmResponse {
	hours: number;
	totals: ITelemetryLlmTotals;
	byModel: ITelemetryLlmModelRow[];
	byProvider: ITelemetryLlmProviderRow[];
	byOperation: ITelemetryLlmOperationRow[];
	topErrors: ITelemetryLlmErrorRow[];
	trend: ITelemetryLlmTrendPoint[];
}

export type IAgentBackendId = "claude_code" | "codex" | "github_copilot";

export type IAgentBackendStage = "spawn" | "auth" | "models" | "run" | "stop";

export type IAgentBackendErrorKind =
	| "binary_not_found"
	| "permission_denied"
	| "auth_required"
	| "auth_expired"
	| "timeout"
	| "non_zero_exit"
	| "protocol_error"
	| "unsupported_model"
	| "unknown";

export interface IAgentBackendEventProps {
	backend?: string | null;
	outcome?: string | null;
	stage?: string | null;
	error_kind?: string | null;
	duration_ms?: number | null;
}

export interface IAgentBackendErrorKindCount {
	kind: string;
	count: number;
}

export interface IAgentBackendStats {
	backend: IAgentBackendId;
	label: string;
	calls: number;
	successes: number;
	errors: number;
	stageErrors: number;
	successRate: number | null;
	p95DurationMs: number | null;
	topErrorKinds: IAgentBackendErrorKindCount[];
}

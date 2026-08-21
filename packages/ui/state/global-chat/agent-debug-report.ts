import {
	type CopilotStreamEvent,
	createCopilotStreamParser,
} from "../../components/flowpilot/copilot-stream-parser";
import { FLOWPILOT_DEBUG_ENABLED } from "../../lib/flowpilot-debug";
import { sanitizePotentialFlowScriptTextForPersistence } from "../../lib/flowscript-persistence";

export const AGENT_DEBUG_REPORT_SCHEMA = "flowpilot.run-report/v1" as const;
export const FLOWPILOT_PRODUCTION_METRICS_SCHEMA =
	"flowpilot.generation-metrics/v1" as const;

const MAX_EVENTS = 256;
const MAX_REPORT_BYTES = 512 * 1024;
const MAX_PREVIEW_CHARS = 8 * 1024;
const MAX_EVIDENCE_PREVIEW_CHARS = 32 * 1024;
const MAX_SUMMARY_CHARS = 500;
const MAX_GENERATION_ATTEMPTS = 32;
const MAX_GENERATION_DIAGNOSTIC_KEYS = 32;
const MAX_GENERATION_DIAGNOSTIC_KEY_CHARS = 160;
const MAX_FAILURE_SIGNATURES = 12;
const MAX_FAILURE_MESSAGE_CHARS = 200;
const MAX_FAILURE_CODE_CHARS = 80;
/** Keeps the whole metrics payload inside the ingest endpoint's 8 KB props budget. */
const MAX_FAILURE_PAYLOAD_BYTES = 4096;
/** A quoted run of user prose is generalized; a short quoted identifier is diagnostic and kept. */
const MAX_KEPT_QUOTED_CHARS = 32;
const REPORT_SIZE_CACHE = new WeakMap<IAgentDebugReport, number>();
const GENERATION_CANDIDATE_KEYS = new WeakMap<IAgentDebugReport, string[]>();
const GENERATION_TOOL_EVIDENCE = Symbol("flowpilot-generation-tool-evidence");
const GENERATION_FAILURE = Symbol("flowpilot-generation-failure");
const EVENT_PREVIEWS_NORMALIZED = Symbol("flowpilot-event-previews-normalized");
const FLOWSCRIPT_ARTIFACT_SOURCE_HASH = new WeakMap<IAgentDebugEvent, string>();
const SENSITIVE_KEY =
	/(authorization|cookie|credential|password|passwd|secret|token|api.?key|private.?key|client.?secret|access.?key|signature)/i;
const SECRET_VALUE_SIBLING = /^(?:default|default_value|value)$/i;
const FLOWSCRIPT_WORKSPACE_ENVELOPE =
	/<flowscript_workspace>([\s\S]*?)<\/flowscript_workspace>/g;

export type AgentDebugOutcome =
	| "running"
	| "ok"
	| "partial"
	| "error"
	| "cancelled"
	| "timeout"
	| "interrupted";

export type AgentDebugEventKind =
	| "lifecycle"
	| "plan"
	| "tool"
	| "approval"
	| "bridge"
	| "nested";

export const AGENT_RUN_SUMMARY_STAGE = "run_summary" as const;

/** Structured per-run summary emitted once by the host when a FlowPilot run reaches any terminal path. */
export interface IAgentRunSummary {
	outcome?: string;
	provider?: string;
	model?: string;
	duration_ms?: number;
	phases?: number;
	budget?: Record<string, { used?: number; limit?: number }>;
	diagnostics_by_code?: Record<string, number>;
	retained_draft?: { id?: string; revision?: number } | null;
	review_notes?: number;
	applied_commands?: number;
}

const RUN_SUMMARY_FIELDS = [
	"outcome",
	"provider",
	"model",
	"duration_ms",
	"phases",
	"budget",
	"diagnostics_by_code",
	"retained_draft",
	"review_notes",
	"applied_commands",
] as const;

export interface IAgentDebugEvent {
	id: string;
	kind: AgentDebugEventKind;
	stage: string;
	status?: string;
	terminal_status?: string;
	name?: string;
	request_id?: string;
	parent_request_id?: string;
	timestamp_ms: number;
	started_at_ms?: number;
	ended_at_ms?: number;
	duration_ms?: number;
	summary?: string;
	arguments_preview?: string;
	result_summary?: string;
	result_preview?: string;
	reasoning?: string;
	error?: string;
}

export interface IAgentDebugGenerationAttempt {
	attempt_index: number;
	elapsed_ms: number;
	parse_valid: boolean;
	typed_valid: boolean;
	reconcile_valid: boolean;
	accepted: boolean;
	diagnostic_keys?: string[];
}

/**
 * Stable failure taxonomy for the admin FlowPilot trace. Every value is a fixed identifier chosen
 * here, never a string read out of a payload.
 */
export type FlowPilotFailureKind =
	/** A delegated specialist never produced a usable result (dispatch, transport, or timeout). */
	| "subagent_dispatch"
	/** Authoring, validating, committing or applying generated FlowScript failed. */
	| "flowscript_apply"
	/** A delegated UI build failed, or its components could not be applied to a page. */
	| "widget_apply"
	/** A Data Studio / storage / graph operation failed. */
	| "data_apply"
	/** A page or live-surface interaction failed. */
	| "page_apply"
	/** Any other tool call that ended in a non-recoverable status. */
	| "tool_error"
	/** The run itself terminated in a failure outcome. */
	| "run_error";

/**
 * One deduplicated failure cause.
 *
 * `tool` is an allow-listed FlowPilot tool name — unknown and user-authored tool names collapse to
 * `"other"`. `code` is a stable status/diagnostic identifier. `message` is the failure text after
 * secret redaction AND generalization: emails, URLs, identifiers, paths, long digit runs and long
 * quoted strings are replaced with placeholders before the payload leaves the client, so what is
 * retained describes the failure mode and not the workflow it happened in.
 */
export interface IFlowPilotFailureSignature {
	kind: FlowPilotFailureKind;
	tool?: string;
	code?: string;
	message?: string;
	count: number;
}

/**
 * Privacy-safe production telemetry for workflow generation. Every property after the schema is an
 * aggregate counter, except `failures`, which carries bounded, redacted and generalized failure
 * causes so admins can see WHY runs fail. This payload contains no run/message ids, timestamps,
 * model or provider names, prompts, FlowScript source, tool arguments/results, or board ids.
 */
export interface IFlowPilotProductionMetrics {
	schema: typeof FLOWPILOT_PRODUCTION_METRICS_SCHEMA;
	runs_started: number;
	runs_succeeded: number;
	runs_failed: number;
	runs_cancelled: number;
	plans_assessed: number;
	plans_feasible: number;
	plans_infeasible: number;
	attempts_total: number;
	attempts_parse_valid: number;
	attempts_typed_valid: number;
	attempts_reconcile_valid: number;
	attempts_applied: number;
	queued_reviews: number;
	apply_dispositions: number;
	dismissed_dispositions: number;
	stale_dispositions: number;
	error_dispositions: number;
	diagnostic_occurrences: number;
	repeated_diagnostic_occurrences: number;
	validation_regressions: number;
	boards_inspected: number;
	empty_boards_after_run: number;
	failures_total: number;
	subagent_dispatch_failures: number;
	flowscript_apply_failures: number;
	widget_apply_failures: number;
	data_apply_failures: number;
	page_apply_failures: number;
	tool_failures: number;
	run_failures: number;
	/** Deduplicated, redacted failure causes, bounded in both count and serialized bytes. */
	failures: IFlowPilotFailureSignature[];
}

export type AgentGenerationReviewDisposition =
	| "applied"
	| "dismissed"
	| "stale"
	| "error";

/**
 * Production evidence matching `flowpilot.generation-evaluation/v1` in the core evaluator.
 * It deliberately contains only booleans, elapsed time and stable diagnostic identifiers: tool
 * payloads and authored FlowScript remain in the separately bounded/redacted previews.
 */
export interface IAgentDebugGenerationEvaluation {
	version: "flowpilot.generation-evaluation/v1";
	run_id: string;
	status: "running" | "succeeded" | "failed" | "cancelled";
	plan_outcome: "not_assessed" | "feasible" | "infeasible";
	final_board_node_count?: number;
	attempts: IAgentDebugGenerationAttempt[];
}

export interface IAgentDebugReport {
	schema: typeof AGENT_DEBUG_REPORT_SCHEMA;
	message_id: string;
	started_at_ms: number;
	ended_at_ms?: number;
	duration_ms?: number;
	outcome: AgentDebugOutcome;
	terminal_stage?: string;
	terminal_code?: string;
	summary?: string;
	provider?: string;
	model?: string;
	reasoning_effort?: string;
	input_preview?: string;
	output_preview?: string;
	generation_evaluation?: IAgentDebugGenerationEvaluation;
	events: IAgentDebugEvent[];
	truncation?: {
		events_dropped: number;
		bytes_dropped: number;
	};
}

export interface AgentDebugReportMetadata {
	provider?: string;
	model?: string;
	reasoningEffort?: string;
	startedAtMs?: number;
	inputPreview?: unknown;
}

export function createAgentDebugReport(
	messageId: string,
	metadata: AgentDebugReportMetadata = {},
): IAgentDebugReport {
	const report: IAgentDebugReport = {
		schema: AGENT_DEBUG_REPORT_SCHEMA,
		message_id: messageId,
		started_at_ms: metadata.startedAtMs ?? Date.now(),
		outcome: "running",
		provider: cleanSummary(metadata.provider),
		model: cleanSummary(metadata.model),
		reasoning_effort: cleanSummary(metadata.reasoningEffort),
		input_preview: agentDebugPreview(metadata.inputPreview),
		events: [],
	};
	GENERATION_CANDIDATE_KEYS.set(report, []);
	return report;
}

function cleanSummary(value: unknown, limit = MAX_SUMMARY_CHARS) {
	if (typeof value !== "string") return undefined;
	const cleaned = redactSecretsInText(value).trim();
	if (!cleaned) return undefined;
	return truncate(cleaned, limit);
}

function truncate(value: string, limit: number) {
	return value.length <= limit
		? value
		: `${value.slice(0, Math.max(0, limit - 1))}…`;
}

function truncatePreview(value: string, limit: number) {
	if (value.length <= limit) return value;
	const suffix = "… [truncated]";
	if (limit <= suffix.length) return suffix.slice(0, Math.max(0, limit));
	return `${value.slice(0, Math.max(0, limit - suffix.length))}${suffix}`;
}

function structurallyBoundedJsonPreview(
	value: unknown,
	limit: number,
	allowWorkspaceEnvelopes = true,
) {
	const boundStrings = (candidate: unknown, stringLimit: number): unknown => {
		if (typeof candidate === "string") {
			return truncatePreview(candidate, stringLimit);
		}
		if (Array.isArray(candidate)) {
			return candidate.map((entry) => boundStrings(entry, stringLimit));
		}
		if (candidate && typeof candidate === "object") {
			return Object.fromEntries(
				Object.entries(candidate).map(([key, entry]) => [
					key,
					boundStrings(entry, stringLimit),
				]),
			);
		}
		return candidate;
	};

	try {
		const redacted = redactValue(
			value,
			"",
			0,
			Math.max(0, limit),
			allowWorkspaceEnvelopes,
		);
		let serialized = JSON.stringify(redacted);
		if (serialized.length <= limit) return serialized;

		let stringLimit = Math.max(
			0,
			Math.floor(limit * Math.max(0, limit / serialized.length) * 0.9),
		);
		for (let attempt = 0; attempt < 3; attempt += 1) {
			serialized = JSON.stringify(boundStrings(redacted, stringLimit));
			if (serialized.length <= limit) return serialized;
			if (stringLimit === 0) break;
			const scaled = Math.floor(
				stringLimit * Math.max(0, limit / serialized.length) * 0.9,
			);
			stringLimit = Math.max(0, Math.min(stringLimit - 1, scaled));
		}
	} catch {
		// Fall through to a small, content-free JSON preview.
	}

	const fallback = JSON.stringify({
		__truncated__: "Preview exceeded the structural size limit.",
	});
	return truncatePreview(fallback, limit);
}

function sanitizeFlowScriptAwareText(value: string, limit: number) {
	let cursor = 0;
	let foundEnvelope = false;
	let output = "";
	for (const match of value.matchAll(FLOWSCRIPT_WORKSPACE_ENVELOPE)) {
		foundEnvelope = true;
		const start = match.index ?? 0;
		output += sanitizePotentialFlowScriptTextForPersistence(
			value.slice(cursor, start),
		);
		const open = "<flowscript_workspace>";
		const close = "</flowscript_workspace>";
		try {
			const payload = JSON.parse(match[1] ?? "");
			const jsonBudget = Math.max(0, limit - open.length - close.length);
			output += `${open}${structurallyBoundedJsonPreview(
				payload,
				jsonBudget,
				false,
			)}${close}`;
		} catch {
			output += `${open}[REDACTED malformed workspace payload]${close}`;
		}
		cursor = start + match[0].length;
	}
	if (!foundEnvelope) {
		return sanitizePotentialFlowScriptTextForPersistence(value);
	}
	output += sanitizePotentialFlowScriptTextForPersistence(value.slice(cursor));
	return output;
}

function redactSecretsInText(
	value: string,
	limit = MAX_PREVIEW_CHARS,
	allowWorkspaceEnvelopes = true,
) {
	return (
		(
			allowWorkspaceEnvelopes
				? sanitizeFlowScriptAwareText(value, limit)
				: sanitizePotentialFlowScriptTextForPersistence(value)
		)
			// The FlowScript-aware pass above handles semantic @secret declarations even when
			// their identifiers are innocuous. The remaining heuristics cover unannotated provider
			// text and conventionally sensitive names.
			.replace(
				/(\b(?:const|let|var)\s+\w*(?:password|passwd|secret|token|api_?key|private_?key|client_?secret|access_?key|signature)\w*\s*:\s*[^=;\r\n]+?=\s*)("[^"]*"|'[^']*'|`[^`]*`|[^;\r\n]+)/gi,
				(match, prefix: string, initializer: string) => {
					// The semantic FlowScript pass above has already replaced @secret primitive
					// literals with canonical safe values. Keep those values syntax-valid so a
					// repeated preview pass remains idempotent.
					if (/^(?:""|''|0|false)$/i.test(initializer.trim())) return match;
					return `${prefix}[REDACTED]`;
				},
			)
			.replace(/Bearer\s+[A-Za-z0-9._~+\/-]+/gi, "Bearer [REDACTED]")
			.replace(/Basic\s+[A-Za-z0-9+/=]+/gi, "Basic [REDACTED]")
			.replace(
				/-----BEGIN [^-\r\n]*PRIVATE KEY-----[\s\S]*?-----END [^-\r\n]*PRIVATE KEY-----/gi,
				"[REDACTED PRIVATE KEY]",
			)
			.replace(/((?:set-)?cookie\s*:\s*)[^\r\n]+/gi, "$1[REDACTED]")
			.replace(
				/(\b(?:password|passwd|secret|token|api[\s_-]*key|authorization|client[\s_-]*secret|access[\s_-]*(?:token|key)|refresh[\s_-]*token|private[\s_-]*key|session[\s_-]*key)\s*[=:]\s*)(?![A-Za-z_][A-Za-z0-9_]*(?:\s*\[\s*\])?\s*=)[^,;\s}]+/gi,
				"$1[REDACTED]",
			)
			.replace(
				/([?&](?:X-Amz-(?:Signature|Credential|Security-Token)|X-Goog-(?:Signature|Credential)|GoogleAccessId|AWSAccessKeyId|access[_-]?token|refresh[_-]?token|api[_-]?key|client[_-]?secret|token|signature|sig|secret|credential|code|key)=)[^&#\s]+/gi,
				"$1[REDACTED]",
			)
	);
}

function parseNestedJsonContainer(value: string): unknown | undefined {
	const trimmed = value.trim();
	if (
		!(
			(trimmed.startsWith("{") && trimmed.endsWith("}")) ||
			(trimmed.startsWith("[") && trimmed.endsWith("]"))
		)
	)
		return undefined;
	try {
		return JSON.parse(trimmed);
	} catch {
		return undefined;
	}
}

function redactValue(
	value: unknown,
	key = "",
	depth = 0,
	stringLimit = MAX_PREVIEW_CHARS,
	allowWorkspaceEnvelopes = true,
): unknown {
	if (key === "_flowpilot_image_urls") {
		const count = Array.isArray(value) ? value.length : 0;
		return `[OMITTED ${count} IMAGE ATTACHMENT${count === 1 ? "" : "S"}]`;
	}
	if (SENSITIVE_KEY.test(key)) return "[REDACTED]";
	// MCP content -> text -> JSON adds two legitimate wrapper levels before the diagnostic list.
	// Keep enough depth for stable diagnostic codes while arrays/objects remain independently
	// bounded below.
	if (depth > 8) return "[TRUNCATED_DEPTH]";
	if (typeof value === "string") {
		// MCP text-content envelopes frequently carry the actual tool result as a second JSON
		// document. Parse that document before FlowScript-aware sanitization so a fail-closed
		// redaction of its `source` field cannot erase sibling status/diagnostic evidence.
		const nestedJson = parseNestedJsonContainer(value);
		if (nestedJson !== undefined) {
			return redactValue(
				nestedJson,
				key,
				depth + 1,
				stringLimit,
				allowWorkspaceEnvelopes,
			);
		}
		return truncatePreview(
			redactSecretsInText(value, stringLimit, allowWorkspaceEnvelopes),
			stringLimit,
		);
	}
	if (value === null || typeof value === "number" || typeof value === "boolean")
		return value;
	if (Array.isArray(value)) {
		const redacted = value
			.slice(0, 25)
			.map((entry) =>
				redactValue(
					entry,
					key,
					depth + 1,
					stringLimit,
					allowWorkspaceEnvelopes,
				),
			);
		if (value.length > 25) {
			redacted.push(`[TRUNCATED ${value.length - 25} ITEMS]`);
		}
		return redacted;
	}
	if (typeof value === "object") {
		const record = value as Record<string, unknown>;
		const entries = Object.entries(record);
		const hasSecretSibling = record.secret === true;
		const redacted = Object.fromEntries(
			entries
				.slice(0, 50)
				.map(([childKey, childValue]) => [
					childKey,
					hasSecretSibling && SECRET_VALUE_SIBLING.test(childKey)
						? "[REDACTED]"
						: redactValue(
								childValue,
								childKey,
								depth + 1,
								stringLimit,
								allowWorkspaceEnvelopes,
							),
				]),
		);
		if (entries.length > 50) {
			redacted.__truncated__ = `${entries.length - 50} fields omitted`;
		}
		return redacted;
	}
	return String(value);
}

/** Bounded, deterministic and secret-redacted JSON preview for persisted diagnostics. */
export function agentDebugPreview(value: unknown, limit = MAX_PREVIEW_CHARS) {
	if (value === undefined) return undefined;
	if (typeof value === "string") {
		const trimmed = value.trim();
		if (
			(trimmed.startsWith("{") && trimmed.endsWith("}")) ||
			(trimmed.startsWith("[") && trimmed.endsWith("]"))
		) {
			try {
				return structurallyBoundedJsonPreview(JSON.parse(trimmed), limit);
			} catch {
				// Fall back to text redaction for malformed/provider-specific JSON fragments.
			}
		}
		return truncatePreview(redactSecretsInText(value, limit), limit);
	}
	try {
		return structurallyBoundedJsonPreview(value, limit);
	} catch {
		return truncatePreview(redactSecretsInText(String(value)), limit);
	}
}

type PreNormalizedEvent = IAgentDebugEvent & {
	[EVENT_PREVIEWS_NORMALIZED]?: true;
};

/** Builder events already carry bounded, redacted previews — skip the second redaction pass. */
function markEventPreviewsNormalized<T extends IAgentDebugEvent>(event: T): T {
	(event as PreNormalizedEvent)[EVENT_PREVIEWS_NORMALIZED] = true;
	return event;
}

function normalizeEvent(event: IAgentDebugEvent): IAgentDebugEvent {
	const startedAt = event.started_at_ms;
	const endedAt = event.ended_at_ms;
	const duration =
		event.duration_ms ??
		(startedAt !== undefined && endedAt !== undefined
			? Math.max(0, endedAt - startedAt)
			: undefined);
	const normalized: IAgentDebugEvent = (event as PreNormalizedEvent)[
		EVENT_PREVIEWS_NORMALIZED
	]
		? { ...event, duration_ms: duration }
		: (() => {
				const previewLimit =
					event.stage === "artifact" ||
					(event.kind === "nested" &&
						(event.stage === "nested_run_started" ||
							event.stage === "nested_run_finished"))
						? MAX_EVIDENCE_PREVIEW_CHARS
						: MAX_PREVIEW_CHARS;
				return {
					...event,
					summary: cleanSummary(event.summary),
					arguments_preview: agentDebugPreview(
						event.arguments_preview,
						previewLimit,
					),
					result_summary: cleanSummary(event.result_summary),
					result_preview: agentDebugPreview(event.result_preview, previewLimit),
					reasoning: agentDebugPreview(event.reasoning),
					error: cleanSummary(event.error, MAX_PREVIEW_CHARS),
					duration_ms: duration,
				};
			})();
	return Object.fromEntries(
		Object.entries(normalized).filter(([, value]) => value !== undefined),
	) as unknown as IAgentDebugEvent;
}

function reportSize(report: IAgentDebugReport) {
	const cached = REPORT_SIZE_CACHE.get(report);
	if (cached !== undefined) return cached;
	try {
		const size = new TextEncoder().encode(JSON.stringify(report)).byteLength;
		REPORT_SIZE_CACHE.set(report, size);
		return size;
	} catch {
		return MAX_REPORT_BYTES + 1;
	}
}

function withReportTruncation(
	report: IAgentDebugReport,
	truncation: NonNullable<IAgentDebugReport["truncation"]>,
) {
	const next = { ...report, truncation };
	const previous = report.truncation;
	const sizeDelta = previous
		? valueSize(truncation) - valueSize(previous)
		: valueSize({ truncation }) - 1;
	REPORT_SIZE_CACHE.set(next, reportSize(report) + sizeDelta);
	const candidateKeys = GENERATION_CANDIDATE_KEYS.get(report);
	if (candidateKeys) GENERATION_CANDIDATE_KEYS.set(next, candidateKeys);
	return next;
}

function valueSize(value: unknown) {
	try {
		return new TextEncoder().encode(JSON.stringify(value)).byteLength;
	} catch {
		return 0;
	}
}

function isNestedRunEvidence(event: IAgentDebugEvent) {
	return (
		event.kind === "nested" &&
		(event.stage === "nested_run_started" ||
			event.stage === "nested_run_finished")
	);
}

function isRunSummaryEvent(event: IAgentDebugEvent) {
	return event.stage === AGENT_RUN_SUMMARY_STAGE;
}

/** Higher values contain more useful evidence when the report has to shed history. */
function retentionPriority(event: IAgentDebugEvent) {
	// One tiny structured summary replaces forensic archaeology across the whole run; it is never
	// compacted or dropped under pressure.
	if (isRunSummaryEvent(event)) return 6;
	if (isNestedRunEvidence(event)) return 5;
	if (event.stage === "artifact" || event.kind === "approval") return 4;
	if (
		event.stage === "tool_end" ||
		event.stage.startsWith("request_") ||
		(event.ended_at_ms !== undefined && event.status !== "progress")
	)
		return 3;
	if (
		event.stage === "plan" ||
		event.stage === "tool_progress" ||
		event.status === "progress"
	)
		return 0;
	return 1;
}

function compactEventDetails(event: IAgentDebugEvent, limit: number) {
	let changed = false;
	const compacted = { ...event };
	for (const key of [
		"arguments_preview",
		"result_preview",
		"reasoning",
		"error",
	] as const) {
		const value = compacted[key];
		if (typeof value !== "string" || value.length <= limit) continue;
		compacted[key] = truncatePreview(value, limit);
		changed = true;
	}
	return changed ? compacted : event;
}

function replaceEventsForPressure(
	report: IAgentDebugReport,
	events: IAgentDebugEvent[],
	eventsDropped = 0,
) {
	const previousSize = reportSize(report);
	const replaced = { ...report, events };
	const candidateKeys = GENERATION_CANDIDATE_KEYS.get(report);
	if (candidateKeys) GENERATION_CANDIDATE_KEYS.set(replaced, candidateKeys);
	const bytesRemoved = Math.max(0, previousSize - reportSize(replaced));
	return withReportTruncation(replaced, {
		events_dropped: (report.truncation?.events_dropped ?? 0) + eventsDropped,
		bytes_dropped: (report.truncation?.bytes_dropped ?? 0) + bytesRemoved,
	});
}

/**
 * Keep the report bounded while retaining the events most useful for reconstructing a nested run.
 * Progress/plan details are compacted and removed first. Terminal tool, approval, artifact and
 * nested-boundary evidence is only compacted when lower-value history is no longer sufficient.
 */
function compactReportToLimit(report: IAgentDebugReport) {
	let next = report;
	if (reportSize(next) <= MAX_REPORT_BYTES) return next;

	const shrinkPriority = (priority: number, limit: number) => {
		while (true) {
			const currentSize = reportSize(next);
			if (currentSize <= MAX_REPORT_BYTES) return;
			const targetSavings = currentSize - MAX_REPORT_BYTES + 256;
			let estimatedSavings = 0;
			let changed = false;
			const events = [...next.events];
			for (let index = 0; index < events.length; index += 1) {
				const event = events[index];
				if (!event || retentionPriority(event) !== priority) continue;
				const compacted = compactEventDetails(event, limit);
				if (compacted === event) continue;
				events[index] = compacted;
				estimatedSavings += Math.max(
					0,
					valueSize(event) - valueSize(compacted),
				);
				changed = true;
				if (estimatedSavings >= targetSavings) break;
			}
			if (!changed) return;
			const previousSize = currentSize;
			next = replaceEventsForPressure(next, events);
			if (reportSize(next) >= previousSize) return;
		}
	};
	const dropPriority = (priority: number) => {
		while (true) {
			const currentSize = reportSize(next);
			if (currentSize <= MAX_REPORT_BYTES) return;
			const targetSavings = currentSize - MAX_REPORT_BYTES + 256;
			let estimatedSavings = 0;
			const removedIndexes = new Set<number>();
			for (let index = 0; index < next.events.length; index += 1) {
				const event = next.events[index];
				if (!event || retentionPriority(event) !== priority) continue;
				removedIndexes.add(index);
				estimatedSavings += valueSize(event) + 1;
				if (estimatedSavings >= targetSavings) break;
			}
			if (removedIndexes.size === 0) return;
			const previousSize = currentSize;
			next = replaceEventsForPressure(
				next,
				next.events.filter((_, index) => !removedIndexes.has(index)),
				removedIndexes.size,
			);
			if (reportSize(next) >= previousSize) return;
		}
	};

	shrinkPriority(0, 512);
	dropPriority(0);
	shrinkPriority(1, 1_024);
	dropPriority(1);
	// Artifact snapshots shed payload first and are dropped before terminal evidence: terminal
	// tool_end/lifecycle events carry the diagnostics that explain a failed run and must survive
	// the longest (nested run boundaries excepted).
	shrinkPriority(4, 8 * 1024);
	shrinkPriority(3, 4 * 1024);
	shrinkPriority(4, 2 * 1024);
	shrinkPriority(3, 1_024);
	dropPriority(4);
	dropPriority(3);

	// Nested run boundaries are the last-resort payload to compact and the last events removed.
	shrinkPriority(5, 16 * 1024);
	shrinkPriority(5, 8 * 1024);
	shrinkPriority(5, 2 * 1024);
	shrinkPriority(5, 512);
	if (reportSize(next) <= MAX_REPORT_BYTES) return next;

	// Metadata for 256 boundary events normally remains comfortably below the cap. Keep at least the
	// newest boundary if an unexpectedly large provider envelope still exceeds it.
	while (reportSize(next) > MAX_REPORT_BYTES && next.events.length > 1) {
		const removableIndex = next.events.findIndex(
			(event) => !isNestedRunEvidence(event) && !isRunSummaryEvent(event),
		);
		const index = removableIndex >= 0 ? removableIndex : 0;
		next = replaceEventsForPressure(
			next,
			[...next.events.slice(0, index), ...next.events.slice(index + 1)],
			1,
		);
	}
	return next;
}

function fnv1aHex(value: string) {
	let hash = 0x811c9dc5;
	for (let index = 0; index < value.length; index += 1) {
		hash ^= value.charCodeAt(index);
		hash = Math.imul(hash, 0x01000193);
	}
	return (hash >>> 0).toString(16).padStart(8, "0");
}

/**
 * FlowScript workspace artifacts re-embed the complete source per validation snapshot. Keep the
 * first copy of each distinct source and replace repeats with a reference line so a long repair
 * loop cannot spend the entire report budget on identical multi-KB sources. Status, revision and
 * diagnostics of the repeated snapshot are kept in full.
 */
function dedupeFlowScriptArtifactSource(
	report: IAgentDebugReport,
	event: IAgentDebugEvent,
): { event: IAgentDebugEvent; sourceHash?: string } {
	if (event.stage !== "artifact" || event.name !== "flowscript_workspace") {
		return { event };
	}
	const preview = event.result_preview;
	if (typeof preview !== "string") return { event };
	let payload: Record<string, unknown>;
	try {
		const parsed = JSON.parse(preview);
		if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
			return { event };
		}
		payload = parsed as Record<string, unknown>;
	} catch {
		return { event };
	}
	const source = payload.source;
	if (typeof source !== "string" || source.length === 0) return { event };
	const sourceHash = fnv1aHex(source);
	const seen = report.events.some(
		(existing) => FLOWSCRIPT_ARTIFACT_SOURCE_HASH.get(existing) === sourceHash,
	);
	if (!seen) return { event, sourceHash };
	const reference = `[FlowScript source unchanged: ${source.split(/\r?\n/).length} lines, ${source.length} chars, hash ${sourceHash} — full copy retained on the first artifact with this hash]`;
	return {
		event: {
			...event,
			result_preview: JSON.stringify({ ...payload, source: reference }),
		},
		sourceHash,
	};
}

/**
 * Append or merge an event by id. New events are capped and the report is byte-bounded so a noisy
 * provider cannot make chat history unbounded. Terminal updates to an existing event are retained
 * even after the cap is reached.
 */
export function recordAgentDebugEvent(
	report: IAgentDebugReport,
	event: IAgentDebugEvent,
): IAgentDebugReport {
	const incomingEvidence = (event as GenerationEvidenceEvent)[
		GENERATION_TOOL_EVIDENCE
	];
	const deduped = dedupeFlowScriptArtifactSource(report, event);
	const normalized = normalizeEvent(deduped.event);
	const index = report.events.findIndex((existing) => existing.id === event.id);
	let reportWithTruncation = report;
	let events: IAgentDebugEvent[];
	let eventsSizeDelta: number;
	if (index >= 0) {
		events = [...report.events];
		// Both sides are already normalized. Preserve the earlier bounded previews as-is and only
		// derive duration after the defined-field merge; previewing them again can turn a deliberately
		// truncated JSON value back into malformed wrapper text.
		const merged = { ...events[index], ...normalized };
		const duration =
			merged.duration_ms ??
			(merged.started_at_ms !== undefined && merged.ended_at_ms !== undefined
				? Math.max(0, merged.ended_at_ms - merged.started_at_ms)
				: undefined);
		events[index] =
			duration === undefined ? merged : { ...merged, duration_ms: duration };
		eventsSizeDelta =
			valueSize(events[index]) - valueSize(report.events[index]);
	} else if (report.events.length < MAX_EVENTS) {
		events = [...report.events, normalized];
		eventsSizeDelta = valueSize(normalized) + 1;
	} else {
		const incomingPriority = retentionPriority(normalized);
		let removableIndex = report.events.findIndex(
			(existing) => retentionPriority(existing) < incomingPriority,
		);
		if (removableIndex < 0 && incomingPriority >= 3) {
			removableIndex = report.events.findIndex(
				(existing) => retentionPriority(existing) === incomingPriority,
			);
		}
		if (removableIndex < 0) {
			return withReportTruncation(report, {
				events_dropped: (report.truncation?.events_dropped ?? 0) + 1,
				bytes_dropped:
					(report.truncation?.bytes_dropped ?? 0) + valueSize(normalized),
			});
		}
		const removed = report.events[removableIndex];
		events = [
			...report.events.slice(0, removableIndex),
			...report.events.slice(removableIndex + 1),
			normalized,
		];
		eventsSizeDelta = valueSize(normalized) - valueSize(removed);
		reportWithTruncation = withReportTruncation(report, {
			events_dropped: (report.truncation?.events_dropped ?? 0) + 1,
			bytes_dropped:
				(report.truncation?.bytes_dropped ?? 0) + valueSize(removed),
		});
	}
	if (deduped.sourceHash) {
		const stored = index >= 0 ? events[index] : events[events.length - 1];
		if (stored) FLOWSCRIPT_ARTIFACT_SOURCE_HASH.set(stored, deduped.sourceHash);
	}
	const evaluationEvent =
		index >= 0 ? events[index] : (events[events.length - 1] ?? normalized);
	const evidence =
		incomingEvidence ?? generationEvidenceForRecordedEvent(evaluationEvent);
	const generation = updateGenerationEvaluation(
		report,
		evaluationEvent,
		evidence,
	);
	const evaluationDelta =
		generation.evaluation === report.generation_evaluation
			? 0
			: (generation.evaluation ? valueSize(generation.evaluation) + 32 : 0) -
				(report.generation_evaluation
					? valueSize(report.generation_evaluation)
					: 0);
	const next: IAgentDebugReport = {
		...reportWithTruncation,
		events,
		generation_evaluation: generation.evaluation,
	};
	GENERATION_CANDIDATE_KEYS.set(next, generation.candidateKeys);
	// Incremental, slightly conservative size accounting: a full-report JSON.stringify per recorded
	// event is O(report) and dominated this hot path. The exact size is re-measured only when the
	// running estimate crosses the cap, immediately before evidence would be shed.
	REPORT_SIZE_CACHE.set(
		next,
		reportSize(reportWithTruncation) + eventsSizeDelta + evaluationDelta + 16,
	);
	if (reportSize(next) <= MAX_REPORT_BYTES) return next;
	REPORT_SIZE_CACHE.delete(next);
	const compacted = compactReportToLimit(next);
	if (compacted !== next) {
		GENERATION_CANDIDATE_KEYS.set(compacted, generation.candidateKeys);
	}
	return compacted;
}

export function finalizeAgentDebugReport(
	report: IAgentDebugReport,
	options: {
		outcome: AgentDebugOutcome;
		terminalStage: string;
		terminalCode?: string;
		summary?: string;
		outputPreview?: unknown;
		endedAtMs?: number;
		finalBoardNodeCount?: number;
	},
): IAgentDebugReport {
	const endedAt = options.endedAtMs ?? Date.now();
	const finalized = {
		...report,
		ended_at_ms: endedAt,
		duration_ms: Math.max(0, endedAt - report.started_at_ms),
		outcome: options.outcome,
		terminal_stage: cleanSummary(options.terminalStage),
		terminal_code: cleanSummary(options.terminalCode),
		summary: cleanSummary(options.summary, MAX_PREVIEW_CHARS),
		output_preview: agentDebugPreview(options.outputPreview),
		generation_evaluation: report.generation_evaluation
			? {
					...report.generation_evaluation,
					status: finalizedGenerationStatus(options.outcome),
					...(Number.isSafeInteger(options.finalBoardNodeCount) &&
					(options.finalBoardNodeCount ?? -1) >= 0
						? { final_board_node_count: options.finalBoardNodeCount }
						: {}),
				}
			: undefined,
	};
	const candidateKeys = GENERATION_CANDIDATE_KEYS.get(report);
	if (candidateKeys) GENERATION_CANDIDATE_KEYS.set(finalized, candidateKeys);
	const compacted = compactReportToLimit(finalized);
	if (candidateKeys && compacted !== finalized) {
		GENERATION_CANDIDATE_KEYS.set(compacted, candidateKeys);
	}
	return compacted;
}

/**
 * Mark a restored report whose run is no longer alive. A checkpoint persisted mid-run keeps
 * `outcome: "running"` forever otherwise; restore paths call this so dead runs render and export
 * as interrupted, ending at their last recorded event instead of appearing live. Reports with a
 * real terminal outcome pass through unchanged.
 */
export function markAgentDebugReportInterrupted(
	report: IAgentDebugReport,
): IAgentDebugReport {
	if (report.outcome !== "running") return report;
	let lastEventMs = report.started_at_ms;
	for (const event of report.events) {
		lastEventMs = Math.max(
			lastEventMs,
			event.timestamp_ms,
			event.ended_at_ms ?? 0,
		);
	}
	return {
		...report,
		outcome: "interrupted",
		ended_at_ms: lastEventMs,
		duration_ms: Math.max(0, lastEventMs - report.started_at_ms),
		terminal_stage: report.terminal_stage ?? "interrupted",
		terminal_code: report.terminal_code ?? "RUN_INTERRUPTED",
		summary:
			report.summary ??
			"The run is no longer active. This restored report ends at its last recorded event; the final outcome was never written.",
		...(report.generation_evaluation
			? {
					generation_evaluation: {
						...report.generation_evaluation,
						status: "failed" as const,
					},
				}
			: {}),
	};
}

function eventRecord(data: unknown) {
	return data && typeof data === "object"
		? (data as Record<string, unknown>)
		: {};
}

function normalizedTerminalStatus(value: unknown): {
	status: "progress" | "done" | "partial" | "error" | "timeout";
	terminalStatus?: string;
} {
	const terminalStatus = cleanSummary(value);
	const normalized = terminalStatus?.toLowerCase() ?? "done";
	if (["timeout", "timed_out"].includes(normalized)) {
		return { status: "timeout", terminalStatus };
	}
	if (
		[
			"error",
			"failed",
			"failure",
			"validation_error",
			"validation_errors",
		].includes(normalized)
	) {
		return { status: "error", terminalStatus };
	}
	if (["partial", "denied", "cancelled", "canceled"].includes(normalized)) {
		return { status: "partial", terminalStatus };
	}
	if (["running", "pending", "submitted", "in_progress"].includes(normalized)) {
		return { status: "progress", terminalStatus };
	}
	if (
		!terminalStatus ||
		["ok", "success", "done", "completed", "queued", "no_changes"].includes(
			normalized,
		)
	) {
		return { status: "done", terminalStatus };
	}
	// An unrecognised provider status is evidence of an incomplete/ambiguous outcome, not success.
	return { status: "partial", terminalStatus };
}

interface GenerationToolEvidence {
	kind: "plan" | "candidate" | "final" | "disposition";
	planOutcome?: IAgentDebugGenerationEvaluation["plan_outcome"];
	finalBoardNodeCount?: number;
	candidateKey?: string;
	parseValid?: boolean;
	typedValid?: boolean;
	reconcileValid?: boolean;
	accepted?: boolean;
	reviewQueued?: boolean;
	disposition?: AgentGenerationReviewDisposition;
	diagnosticKeys?: string[];
}

type GenerationEvidenceEvent = IAgentDebugEvent & {
	[GENERATION_TOOL_EVIDENCE]?: GenerationToolEvidence;
};

function parseJsonRecord(
	value: unknown,
	depth = 0,
): Record<string, unknown> | undefined {
	if (depth > 4) return undefined;
	if (typeof value === "string") {
		const parsed = parseNestedJsonContainer(value);
		return parsed === undefined
			? undefined
			: parseJsonRecord(parsed, depth + 1);
	}
	if (Array.isArray(value)) {
		for (const entry of value) {
			const parsed = parseJsonRecord(entry, depth + 1);
			if (parsed) return parsed;
		}
		return undefined;
	}
	if (!value || typeof value !== "object") return undefined;
	const record = value as Record<string, unknown>;
	if (record.type === "text" && typeof record.text === "string") {
		const parsedText = parseJsonRecord(record.text, depth + 1);
		if (parsedText) return parsedText;
	}
	if (Array.isArray(record.content)) {
		const parsedContent = parseJsonRecord(record.content, depth + 1);
		if (parsedContent) return parsedContent;
	}
	return record;
}

function taggedJson(value: unknown, tag: string): unknown {
	if (typeof value !== "string") return undefined;
	const match = value.match(new RegExp(`<${tag}>([\\s\\S]*?)<\\/${tag}>`, "i"));
	if (!match?.[1]) return undefined;
	try {
		return JSON.parse(match[1]);
	} catch {
		return undefined;
	}
}

function stableDiagnosticKeys(
	diagnostics: unknown,
	topLevelCode?: unknown,
): string[] {
	const candidates: unknown[] = [];
	if (Array.isArray(diagnostics)) {
		for (const diagnostic of diagnostics) {
			const record = eventRecord(diagnostic);
			candidates.push(record.code ?? record.id);
		}
	}
	const keys = new Set<string>();
	for (const candidate of candidates) {
		if (typeof candidate !== "string") continue;
		const key = cleanSummary(candidate, MAX_GENERATION_DIAGNOSTIC_KEY_CHARS);
		if (!key || !/^[A-Za-z][A-Za-z0-9_.:-]*$/.test(key)) continue;
		keys.add(key);
		if (keys.size >= MAX_GENERATION_DIAGNOSTIC_KEYS) break;
	}
	// A wrapper code (for example IR_DRAFT_INVALID) is useful when a schema failure has no
	// structured diagnostics. When root diagnostics do exist, including both makes one failure look
	// like two repeated causes and weakens repair-loop analysis.
	if (keys.size === 0 && typeof topLevelCode === "string") {
		const key = cleanSummary(topLevelCode, MAX_GENERATION_DIAGNOSTIC_KEY_CHARS);
		if (key && /^[A-Za-z][A-Za-z0-9_.:-]*$/.test(key)) keys.add(key);
	}
	return [...keys];
}

function diagnosticPhases(diagnostics: unknown) {
	if (!Array.isArray(diagnostics)) return [];
	return diagnostics
		.map((diagnostic) => eventRecord(diagnostic).phase)
		.filter((phase): phase is string => typeof phase === "string")
		.map((phase) => phase.toLowerCase().replaceAll("_", ""));
}

function generationEvidenceForToolEnd(
	name: string | undefined,
	rawResult: unknown,
): GenerationToolEvidence | undefined {
	const normalizedName = name?.toLowerCase() ?? "";
	const directPayload = parseJsonRecord(rawResult);
	if (normalizedName.endsWith("flowpilot_board")) {
		const count = directPayload?.final_board_node_count;
		if (
			typeof count === "number" &&
			Number.isSafeInteger(count) &&
			count >= 0
		) {
			return {
				kind: "final",
				finalBoardNodeCount: count,
			};
		}
	}
	if (normalizedName.endsWith("plan_flow_ir")) {
		return {
			kind: "plan",
			planOutcome:
				typeof directPayload?.feasible === "boolean"
					? directPayload.feasible
						? "feasible"
						: "infeasible"
					: "not_assessed",
		};
	}

	const isTypedCommit = normalizedName.endsWith("commit_flow_ir_draft");
	const typed = [
		"begin_flow_ir_draft",
		"update_flow_ir_draft",
		"upsert_flow_ir_module",
		"validate_flow_ir_draft",
		"commit_flow_ir_draft",
	].some((toolName) => normalizedName.endsWith(toolName));
	const raw = [
		"edit_flowscript",
		"write_flowscript",
		"patch_flowscript",
		"check_flowscript",
		"commit_flowscript",
	].some((toolName) => normalizedName.endsWith(toolName));
	if (!typed && !raw) return undefined;

	if (typed) {
		// Core/Bits embeds a compact legacy Flow IR result alongside the workspace/commands tags. SDK
		// adapters return the same shape as plain JSON.
		const payload =
			directPayload ??
			(isTypedCommit
				? parseJsonRecord(taggedJson(rawResult, "typed_commit_result"))
				: undefined);
		const diagnostics = payload?.diagnostics;
		const phases = diagnosticPhases(diagnostics);
		const status = String(payload?.status ?? "").toLowerCase();
		const code = payload?.code;
		const schemaInvalid =
			typeof code === "string" &&
			[
				"IR_DRAFT_INVALID",
				"IR_DRAFT_UPDATE_INVALID",
				"IR_MODULE_INVALID",
				"IR_DRAFT_VALIDATION_INVALID",
				"IR_COMMIT_INVALID",
			].includes(code);
		const keys = stableDiagnosticKeys(diagnostics, code);
		const legacyWorkspace = eventRecord(
			taggedJson(rawResult, "flowscript_workspace"),
		);
		const legacyCommands = taggedJson(rawResult, "commands");
		const legacyCommitQueued =
			isTypedCommit &&
			String(legacyWorkspace.status ?? "").toLowerCase() === "queued" &&
			Array.isArray(legacyCommands) &&
			legacyCommands.length > 0;
		const parseValid =
			(Boolean(payload) || legacyCommitQueued) &&
			!schemaInvalid &&
			!phases.includes("parse") &&
			!keys.includes("FS_PARSE_ERROR");
		const typedInvalidPhases = new Set([
			"parse",
			"compile",
			"render",
			"draft",
			"capability",
			"catalogresolution",
			"typecheck",
		]);
		const typedValid =
			parseValid && !phases.some((phase) => typedInvalidPhases.has(phase));
		const missingModules = Array.isArray(payload?.missing_modules)
			? payload.missing_modules.length
			: 0;
		const plan = eventRecord(payload?.capability_plan);
		// A queued command batch is a review reservation, not an accepted/applied workflow. The host
		// records acceptance only after the exact batch is atomically applied to the live board.
		const reviewQueued =
			status === "queued" || status === "already_queued" || legacyCommitQueued;
		const explicitlyValid =
			[
				"draft_started",
				"draft_updated",
				"module_validated",
				"draft_valid",
			].includes(status) || reviewQueued;
		const draftId = payload?.draft_id;
		const revision = payload?.selected_revision ?? payload?.revision;
		return {
			kind: "candidate",
			candidateKey:
				typeof draftId === "string" &&
				draftId.length > 0 &&
				(typeof revision === "string" || typeof revision === "number")
					? `${draftId}:${String(revision)}`
					: undefined,
			parseValid,
			typedValid,
			reconcileValid:
				typedValid &&
				explicitlyValid &&
				(!Array.isArray(diagnostics) || diagnostics.length === 0) &&
				missingModules === 0 &&
				plan.feasible !== false,
			accepted: false,
			reviewQueued,
			diagnosticKeys: keys,
		};
	}

	const structured =
		directPayload?.structured_diagnostics ??
		taggedJson(rawResult, "structured_diagnostics");
	const workspace =
		eventRecord(directPayload?.flowscript_workspace).status !== undefined
			? eventRecord(directPayload?.flowscript_workspace)
			: eventRecord(taggedJson(rawResult, "flowscript_workspace"));
	const workspaceStatus = String(
		workspace.status ?? directPayload?.status ?? "",
	).toLowerCase();
	const phases = diagnosticPhases(structured);
	const keys = stableDiagnosticKeys(structured, directPayload?.code);
	const parseValid =
		!phases.includes("parse") && !keys.includes("FS_PARSE_ERROR");
	const typedValid =
		parseValid &&
		!phases.some((phase) =>
			["catalogresolution", "typecheck"].includes(phase),
		) &&
		(workspaceStatus !== "validation_errors" || keys.length > 0);
	const reviewQueued = workspaceStatus === "queued";
	return {
		kind: "candidate",
		parseValid,
		typedValid,
		reconcileValid:
			typedValid &&
			keys.length === 0 &&
			["queued", "no_changes"].includes(workspaceStatus),
		accepted: false,
		reviewQueued,
		diagnosticKeys: keys,
	};
}

/**
 * Tools whose failures are worth naming in the admin trace. Anything outside this list — including
 * user-authored MCP tools, whose names can themselves identify a workspace — reports as "other".
 */
const FAILURE_TOOL_KINDS: Record<string, FlowPilotFailureKind | undefined> = {
	plan_flow_ir: "flowscript_apply",
	begin_flow_ir_draft: "flowscript_apply",
	update_flow_ir_draft: "flowscript_apply",
	upsert_flow_ir_module: "flowscript_apply",
	validate_flow_ir_draft: "flowscript_apply",
	commit_flow_ir_draft: "flowscript_apply",
	write_flowscript: "flowscript_apply",
	edit_flowscript: "flowscript_apply",
	patch_flowscript: "flowscript_apply",
	check_flowscript: "flowscript_apply",
	commit_flowscript: "flowscript_apply",
	read_flowscript_source: "flowscript_apply",
	emit_commands: "flowscript_apply",
	get_current_flowscript: "flowscript_apply",
	flowpilot_board: "flowscript_apply",
	flowpilot_widget: "widget_apply",
	ui_inspect: "widget_apply",
	interact_app_page: "page_apply",
	open_app_page: "page_apply",
	set_page_load_event: "page_apply",
	navigate_view: "page_apply",
	data_studio_agent: "data_apply",
	database_tool: "data_apply",
	storage_tool: "data_apply",
	graph_element_tool: "data_apply",
	graph_overlay_tool: "data_apply",
	graph_query_tool: "data_apply",
	ontology_action_tool: "data_apply",
	research_agent: "subagent_dispatch",
	project_scout: "subagent_dispatch",
	catalog_search: "tool_error",
	find_connectable_nodes: "tool_error",
	get_node_details: "tool_error",
	get_declarations: "tool_error",
	list_board_nodes: "tool_error",
	get_unconfigured_nodes: "tool_error",
	search_by_pin: "tool_error",
	execute_event: "tool_error",
	execute_node: "tool_error",
	call_app_event: "tool_error",
	call_app_chat: "tool_error",
	open_app_chat: "tool_error",
	query_execution_logs: "tool_error",
	upsert_event: "tool_error",
	delete_event: "tool_error",
	create_app: "tool_error",
	acquire_app: "tool_error",
	fork_app: "tool_error",
	fork_preview: "tool_error",
	inspect_app: "tool_error",
	list_apps: "tool_error",
	search_apps: "tool_error",
	search_templates: "tool_error",
	get_app_detail: "tool_error",
	get_template_preview: "tool_error",
	describe_app_interface: "tool_error",
	ask_user: "tool_error",
};

const UNKNOWN_FAILURE_TOOL = "other";

/**
 * Generalizers applied in order, after secret redaction. They trade a little specificity for the
 * guarantee that a retained failure message cannot carry an identity, a location, or user prose.
 */
const FAILURE_MESSAGE_GENERALIZERS: readonly (readonly [RegExp, string])[] = [
	[/\b[\w.+-]+@[\w-]+\.[\w.-]{2,}\b/g, "<email>"],
	[/\b[a-z][a-z0-9+.-]*:\/\/\S+/gi, "<url>"],
	[
		/\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/gi,
		"<id>",
	],
	[/\b(?=[a-z0-9_-]*\d)[a-z0-9_-]{16,}\b/gi, "<id>"],
	// Prefixed record ids (`app_1029384756`, `ref-9182736455`). Four digits keeps
	// short diagnostic suffixes like `error_404` or `line_42` readable.
	[/\b[a-z][a-z0-9]*[_-]\d{4,}\b/gi, "<id>"],
	[/(?:[A-Za-z]:)?(?:[\\/][\w.@~-]+){2,}[\\/]?/g, "<path>"],
	[/\b\d[\d_]{5,}\b/g, "<n>"],
];

/** Generalize one quoted run: short identifiers stay readable, long user prose does not. */
function generalizeQuoted(match: string) {
	return match.length - 2 > MAX_KEPT_QUOTED_CHARS ? "<value>" : match;
}

/**
 * Turn arbitrary failure text into a retainable signature. Secrets are stripped with the same pass
 * the debug previews use, then identities, locations and user prose are generalized away.
 */
export function redactFailureMessage(value: unknown): string | undefined {
	const raw =
		typeof value === "string"
			? value
			: value instanceof Error
				? value.message
				: value === undefined || value === null
					? undefined
					: typeof value === "object"
						? undefined
						: String(value);
	if (!raw?.trim()) return undefined;
	let text = redactSecretsInText(raw, MAX_PREVIEW_CHARS, false);
	for (const [pattern, replacement] of FAILURE_MESSAGE_GENERALIZERS) {
		text = text.replace(pattern, replacement);
	}
	text = text
		.replace(/"[^"\r\n]*"|'[^'\r\n]*'|`[^`\r\n]*`/g, generalizeQuoted)
		.replace(/\s+/g, " ")
		.trim();
	return text.length > 0
		? truncate(text, MAX_FAILURE_MESSAGE_CHARS)
		: undefined;
}

/** Stable, low-cardinality codes only: anything free-form is left to the message field. */
function failureCode(value: unknown): string | undefined {
	const code = cleanSummary(value, MAX_FAILURE_CODE_CHARS);
	if (!code) return undefined;
	return /^[A-Za-z][A-Za-z0-9_.:-]*$/.test(code) ? code : undefined;
}

function failureTool(name: string | undefined) {
	const normalized = cleanSummary(name, MAX_FAILURE_CODE_CHARS)?.toLowerCase();
	if (!normalized) return undefined;
	const known = Object.keys(FAILURE_TOOL_KINDS).find(
		(tool) => normalized === tool || normalized.endsWith(`_${tool}`),
	);
	return known ?? UNKNOWN_FAILURE_TOOL;
}

function failureKindForTool(
	tool: string | undefined,
	fallback: FlowPilotFailureKind,
): FlowPilotFailureKind {
	if (!tool) return fallback;
	return FAILURE_TOOL_KINDS[tool] ?? fallback;
}

export interface FlowPilotFailureDetail {
	kind: FlowPilotFailureKind;
	tool?: string;
	code?: string;
	message?: string;
}

type FailureEvent = IAgentDebugEvent & {
	[GENERATION_FAILURE]?: FlowPilotFailureDetail;
};

function attachFailureDetail<T extends IAgentDebugEvent>(
	event: T,
	detail: FlowPilotFailureDetail | undefined,
) {
	if (detail) (event as FailureEvent)[GENERATION_FAILURE] = detail;
	return event;
}

/**
 * Statuses that must never enter the failure trace. Beyond the success vocabulary this covers the
 * two user-driven outcomes — cancelling a run and denying an approval — which are choices, not
 * defects, and would otherwise drown the signal admins are looking for.
 */
const NON_FAILING_STATUSES = new Set([
	"ok",
	"success",
	"done",
	"completed",
	"committed",
	"queued",
	"already_queued",
	"no_changes",
	"applied",
	"awaiting_approval",
	"progress",
	"running",
	"pending",
	"submitted",
	"in_progress",
	"draft_started",
	"draft_updated",
	"draft_valid",
	"module_validated",
	"cancelled",
	"canceled",
	"dismissed",
	"denied",
]);

function isFailingStatus(value: unknown) {
	const status = String(value ?? "")
		.trim()
		.toLowerCase();
	return status.length > 0 && !NON_FAILING_STATUSES.has(status);
}

/**
 * Derive the failure cause of a settled tool/specialist call. Returns undefined for anything that
 * did not fail, so a healthy run contributes no signatures at all.
 */
function failureDetailForToolEnd(options: {
	name: string | undefined;
	status?: unknown;
	error?: unknown;
	rawResult?: unknown;
	summary?: unknown;
	fallbackKind: FlowPilotFailureKind;
	/** Wins over the tool's own domain: the caller knows the call never got there. */
	overrideKind?: FlowPilotFailureKind;
}): FlowPilotFailureDetail | undefined {
	const payload = parseJsonRecord(options.rawResult);
	const payloadStatus = payload?.status;
	const failed =
		options.error !== undefined ||
		isFailingStatus(options.status) ||
		isFailingStatus(payloadStatus);
	if (!failed) return undefined;
	const tool = failureTool(options.name);
	const diagnostics = Array.isArray(payload?.diagnostics)
		? payload.diagnostics
		: undefined;
	const [firstDiagnostic] = stableDiagnosticKeys(diagnostics, payload?.code);
	return {
		kind:
			options.overrideKind ?? failureKindForTool(tool, options.fallbackKind),
		tool,
		code:
			failureCode(payload?.code) ??
			firstDiagnostic ??
			failureCode(options.status) ??
			failureCode(payloadStatus),
		message:
			redactFailureMessage(options.error) ??
			redactFailureMessage(payload?.message) ??
			redactFailureMessage(payload?.note) ??
			redactFailureMessage(
				diagnostics?.find((entry) => typeof entry === "string"),
			) ??
			redactFailureMessage(options.summary),
	};
}

/** Recover the failure cause of an already-recorded event, whether or not it carries a hint. */
function failureDetailForRecordedEvent(
	event: IAgentDebugEvent,
): FlowPilotFailureDetail | undefined {
	const attached = (event as FailureEvent)[GENERATION_FAILURE];
	if (attached) return attached;
	if (event.stage === "nested_run_started" || event.stage === "tool_start") {
		return undefined;
	}
	const settled =
		event.stage === "tool_end" ||
		event.stage === "nested_run_finished" ||
		event.stage === AGENT_RUN_SUMMARY_STAGE;
	if (!settled) return undefined;
	return failureDetailForToolEnd({
		name: event.name,
		status: event.terminal_status ?? event.status,
		error: event.error,
		rawResult: event.result_preview,
		summary: event.result_summary ?? event.summary,
		fallbackKind:
			event.stage === AGENT_RUN_SUMMARY_STAGE
				? "run_error"
				: event.stage === "nested_run_finished"
					? "subagent_dispatch"
					: "tool_error",
	});
}

function updateGenerationEvaluation(
	report: IAgentDebugReport,
	event: IAgentDebugEvent,
	evidence: GenerationToolEvidence | undefined,
) {
	const existing = report.generation_evaluation;
	const candidateKeys = [
		...(GENERATION_CANDIDATE_KEYS.get(report) ??
			existing?.attempts.map((_, index) => `restored:${index}`) ??
			[]),
	];
	if (!evidence) {
		return { evaluation: existing, candidateKeys };
	}
	const evaluation: IAgentDebugGenerationEvaluation = existing
		? {
				...existing,
				attempts: existing.attempts.map((attempt) => ({
					...attempt,
					diagnostic_keys: attempt.diagnostic_keys
						? [...attempt.diagnostic_keys]
						: undefined,
				})),
			}
		: {
				version: "flowpilot.generation-evaluation/v1",
				run_id: cleanSummary(report.message_id, 160) ?? "unknown",
				status: "running",
				plan_outcome: "not_assessed",
				attempts: [],
			};
	if (evidence.kind === "final") {
		if (evidence.finalBoardNodeCount !== undefined) {
			evaluation.final_board_node_count = evidence.finalBoardNodeCount;
		}
		return { evaluation, candidateKeys };
	}
	if (evidence.kind === "plan") {
		if (evidence.planOutcome && evidence.planOutcome !== "not_assessed") {
			evaluation.plan_outcome = evidence.planOutcome;
		}
		return { evaluation, candidateKeys };
	}
	if (evidence.kind === "disposition") {
		let index = evidence.candidateKey
			? candidateKeys.lastIndexOf(evidence.candidateKey)
			: -1;
		if (index < 0) {
			// Raw FlowScript reviews do not carry a legacy Flow IR draft/revision identity.
			// Bind their host disposition to the most recent reconciled candidate that is still
			// awaiting application.
			index = evaluation.attempts.findLastIndex(
				(attempt) => attempt.reconcile_valid && !attempt.accepted,
			);
		}
		if (index >= 0 && evidence.disposition === "applied") {
			const previous = evaluation.attempts[index];
			if (previous) {
				evaluation.attempts[index] = { ...previous, accepted: true };
			}
		}
		return { evaluation, candidateKeys };
	}

	const candidateKey = evidence.candidateKey ?? event.id;
	let index = candidateKeys.indexOf(candidateKey);
	if (index < 0) {
		if (evaluation.attempts.length >= MAX_GENERATION_ATTEMPTS) {
			return { evaluation, candidateKeys };
		}
		index = evaluation.attempts.length;
		candidateKeys.push(candidateKey);
		evaluation.attempts.push({
			attempt_index: index + 1,
			elapsed_ms: Math.max(
				0,
				(event.ended_at_ms ?? event.timestamp_ms) - report.started_at_ms,
			),
			parse_valid: evidence.parseValid ?? false,
			typed_valid: evidence.typedValid ?? false,
			reconcile_valid: evidence.reconcileValid ?? false,
			accepted: evidence.accepted ?? false,
			diagnostic_keys: evidence.diagnosticKeys?.length
				? evidence.diagnosticKeys.slice(0, MAX_GENERATION_DIAGNOSTIC_KEYS)
				: undefined,
		});
		return { evaluation, candidateKeys };
	}

	const previous = evaluation.attempts[index];
	if (!previous) return { evaluation, candidateKeys };
	const diagnosticKeys = new Set([
		...(previous.diagnostic_keys ?? []),
		...(evidence.diagnosticKeys ?? []),
	]);
	evaluation.attempts[index] = {
		...previous,
		parse_valid: previous.parse_valid || (evidence.parseValid ?? false),
		typed_valid: previous.typed_valid || (evidence.typedValid ?? false),
		reconcile_valid:
			previous.reconcile_valid || (evidence.reconcileValid ?? false),
		accepted: previous.accepted || (evidence.accepted ?? false),
		diagnostic_keys:
			diagnosticKeys.size > 0
				? [...diagnosticKeys].slice(0, MAX_GENERATION_DIAGNOSTIC_KEYS)
				: undefined,
	};
	return { evaluation, candidateKeys };
}

function finalizedGenerationStatus(
	outcome: AgentDebugOutcome,
): IAgentDebugGenerationEvaluation["status"] {
	if (outcome === "running") return "running";
	if (outcome === "ok") return "succeeded";
	if (outcome === "cancelled") return "cancelled";
	return "failed";
}

interface ProductionMetricsAccumulator {
	report: IAgentDebugReport;
	queuedReviewKeys: Set<string>;
	dispositionEventIds: Set<string>;
	dispositions: Record<AgentGenerationReviewDisposition, number>;
	failureEventIds: Set<string>;
	failuresByKind: Record<FlowPilotFailureKind, number>;
	/** Deduplicated by (kind, tool, code, message); insertion order breaks count ties. */
	failures: Map<string, IFlowPilotFailureSignature>;
}

const EMPTY_FAILURE_COUNTS: Record<FlowPilotFailureKind, number> = {
	subagent_dispatch: 0,
	flowscript_apply: 0,
	widget_apply: 0,
	data_apply: 0,
	page_apply: 0,
	tool_error: 0,
	run_error: 0,
};

/** Fold one failure into the accumulator, bounded in distinct signatures but never in counts. */
function collectFailure(
	accumulator: ProductionMetricsAccumulator,
	eventId: string,
	detail: FlowPilotFailureDetail,
) {
	if (accumulator.failureEventIds.has(eventId)) return;
	accumulator.failureEventIds.add(eventId);
	accumulator.failuresByKind[detail.kind] += 1;
	const key = `${detail.kind}|${detail.tool ?? ""}|${detail.code ?? ""}|${detail.message ?? ""}`;
	const existing = accumulator.failures.get(key);
	if (existing) {
		existing.count += 1;
		return;
	}
	if (accumulator.failures.size >= MAX_FAILURE_SIGNATURES) return;
	accumulator.failures.set(key, {
		kind: detail.kind,
		tool: detail.tool,
		code: detail.code,
		message: detail.message,
		count: 1,
	});
}

/** Highest-count signatures first, trimmed to the ingest endpoint's props budget. */
function boundedFailureSignatures(
	accumulator: ProductionMetricsAccumulator,
): IFlowPilotFailureSignature[] {
	const ranked = [...accumulator.failures.values()].sort(
		(left, right) => right.count - left.count,
	);
	const kept: IFlowPilotFailureSignature[] = [];
	let bytes = 0;
	for (const signature of ranked) {
		const size = JSON.stringify(signature).length + 1;
		if (bytes + size > MAX_FAILURE_PAYLOAD_BYTES) break;
		bytes += size;
		kept.push(signature);
	}
	return kept;
}

const PRODUCTION_METRICS_RUNS = new Map<string, ProductionMetricsAccumulator>();
const PENDING_PRODUCTION_METRICS: IFlowPilotProductionMetrics[] = [];
const MAX_PENDING_PRODUCTION_METRICS = 64;

export type FlowPilotProductionMetricsSink = (
	metrics: IFlowPilotProductionMetrics,
) => void | Promise<void>;

let productionMetricsSink: FlowPilotProductionMetricsSink | undefined;

function publishFlowPilotProductionMetrics(
	metrics: IFlowPilotProductionMetrics,
) {
	const sink = productionMetricsSink;
	if (!sink) {
		PENDING_PRODUCTION_METRICS.push(metrics);
		if (PENDING_PRODUCTION_METRICS.length > MAX_PENDING_PRODUCTION_METRICS) {
			PENDING_PRODUCTION_METRICS.shift();
		}
		return;
	}
	try {
		const result = sink(metrics);
		if (result && typeof result === "object" && "catch" in result) {
			void result.catch(() => undefined);
		}
	} catch {
		// Metrics are best-effort and must never affect the workflow/application path.
	}
}

/** Register the production telemetry sink. Pending aggregate-only payloads are flushed on attach. */
export function setFlowPilotProductionMetricsSink(
	sink: FlowPilotProductionMetricsSink | undefined,
) {
	productionMetricsSink = sink;
	if (sink) {
		for (const metrics of PENDING_PRODUCTION_METRICS.splice(0)) {
			publishFlowPilotProductionMetrics(metrics);
		}
	}
	return () => {
		if (productionMetricsSink === sink) productionMetricsSink = undefined;
	};
}

function generationEvidenceForRecordedEvent(
	event: IAgentDebugEvent,
): GenerationToolEvidence | undefined {
	const attached = (event as GenerationEvidenceEvent)[GENERATION_TOOL_EVIDENCE];
	if (attached) return attached;
	if (
		event.stage === "tool_end" ||
		(event.stage === "nested_run_finished" &&
			event.name?.toLowerCase().endsWith("flowpilot_board"))
	) {
		return generationEvidenceForToolEnd(event.name, event.result_preview);
	}
	return undefined;
}

/** Start the aggregate-only run collector. Repeated begin calls (for stream resume) are idempotent. */
export function beginAgentGenerationMetrics(
	runKey: string,
	startedAtMs = Date.now(),
) {
	if (PRODUCTION_METRICS_RUNS.has(runKey)) return;
	const report: IAgentDebugReport = {
		schema: AGENT_DEBUG_REPORT_SCHEMA,
		message_id: "redacted",
		started_at_ms: startedAtMs,
		outcome: "running",
		events: [],
		generation_evaluation: {
			version: "flowpilot.generation-evaluation/v1",
			run_id: "redacted",
			status: "running",
			plan_outcome: "not_assessed",
			attempts: [],
		},
	};
	GENERATION_CANDIDATE_KEYS.set(report, []);
	PRODUCTION_METRICS_RUNS.set(runKey, {
		report,
		queuedReviewKeys: new Set(),
		dispositionEventIds: new Set(),
		dispositions: { applied: 0, dismissed: 0, stale: 0, error: 0 },
		failureEventIds: new Set(),
		failuresByKind: { ...EMPTY_FAILURE_COUNTS },
		failures: new Map(),
	});
}

/** Consume one already-redacted metric event without retaining the event or its payload. */
export function recordAgentGenerationMetricEvent(
	runKey: string,
	event: IAgentDebugEvent,
) {
	const accumulator = PRODUCTION_METRICS_RUNS.get(runKey);
	if (!accumulator) return;
	const failure = failureDetailForRecordedEvent(event);
	if (failure) collectFailure(accumulator, event.id, failure);
	const evidence = generationEvidenceForRecordedEvent(event);
	if (!evidence) return;
	if (evidence.reviewQueued) {
		accumulator.queuedReviewKeys.add(evidence.candidateKey ?? event.id);
	}
	if (
		evidence.kind === "disposition" &&
		evidence.disposition &&
		!accumulator.dispositionEventIds.has(event.id)
	) {
		accumulator.dispositionEventIds.add(event.id);
		accumulator.dispositions[evidence.disposition] += 1;
	}
	const generation = updateGenerationEvaluation(
		accumulator.report,
		event,
		evidence,
	);
	const next = {
		...accumulator.report,
		generation_evaluation: generation.evaluation,
	};
	GENERATION_CANDIDATE_KEYS.set(next, generation.candidateKeys);
	accumulator.report = next;
}

function productionMetricsFromAccumulator(
	accumulator: ProductionMetricsAccumulator,
	outcome: AgentDebugOutcome,
): IFlowPilotProductionMetrics {
	const evaluation = accumulator.report.generation_evaluation ?? {
		version: "flowpilot.generation-evaluation/v1" as const,
		run_id: "redacted",
		status: "running" as const,
		plan_outcome: "not_assessed" as const,
		attempts: [],
	};
	const attempts = evaluation.attempts;
	const diagnosticOccurrences = attempts.reduce(
		(total, attempt) => total + (attempt.diagnostic_keys?.length ?? 0),
		0,
	);
	const seenDiagnostics = new Set<string>();
	let repeatedDiagnosticOccurrences = 0;
	let validationRegressions = 0;
	const validationDepth = (attempt: IAgentDebugGenerationAttempt) =>
		!attempt.parse_valid
			? 0
			: !attempt.typed_valid
				? 1
				: !attempt.reconcile_valid
					? 2
					: 3;
	for (let index = 0; index < attempts.length; index += 1) {
		const attempt = attempts[index];
		if (!attempt) continue;
		for (const key of new Set(attempt.diagnostic_keys ?? [])) {
			if (seenDiagnostics.has(key)) repeatedDiagnosticOccurrences += 1;
			seenDiagnostics.add(key);
		}
		const previous = attempts[index - 1];
		if (previous && validationDepth(attempt) < validationDepth(previous)) {
			validationRegressions += 1;
		}
	}
	const status = finalizedGenerationStatus(outcome);
	const boardInspected = evaluation.final_board_node_count !== undefined;
	return {
		schema: FLOWPILOT_PRODUCTION_METRICS_SCHEMA,
		runs_started: 1,
		runs_succeeded: status === "succeeded" ? 1 : 0,
		runs_failed: status === "failed" ? 1 : 0,
		runs_cancelled: status === "cancelled" ? 1 : 0,
		plans_assessed: evaluation.plan_outcome === "not_assessed" ? 0 : 1,
		plans_feasible: evaluation.plan_outcome === "feasible" ? 1 : 0,
		plans_infeasible: evaluation.plan_outcome === "infeasible" ? 1 : 0,
		attempts_total: attempts.length,
		attempts_parse_valid: attempts.filter((attempt) => attempt.parse_valid)
			.length,
		attempts_typed_valid: attempts.filter(
			(attempt) => attempt.parse_valid && attempt.typed_valid,
		).length,
		attempts_reconcile_valid: attempts.filter(
			(attempt) =>
				attempt.parse_valid && attempt.typed_valid && attempt.reconcile_valid,
		).length,
		attempts_applied: attempts.filter(
			(attempt) =>
				attempt.parse_valid &&
				attempt.typed_valid &&
				attempt.reconcile_valid &&
				attempt.accepted,
		).length,
		queued_reviews: accumulator.queuedReviewKeys.size,
		apply_dispositions: accumulator.dispositions.applied,
		dismissed_dispositions: accumulator.dispositions.dismissed,
		stale_dispositions: accumulator.dispositions.stale,
		error_dispositions: accumulator.dispositions.error,
		diagnostic_occurrences: diagnosticOccurrences,
		repeated_diagnostic_occurrences: repeatedDiagnosticOccurrences,
		validation_regressions: validationRegressions,
		boards_inspected: boardInspected ? 1 : 0,
		empty_boards_after_run:
			boardInspected && evaluation.final_board_node_count === 0 ? 1 : 0,
		failures_total: accumulator.failureEventIds.size,
		subagent_dispatch_failures: accumulator.failuresByKind.subagent_dispatch,
		flowscript_apply_failures: accumulator.failuresByKind.flowscript_apply,
		widget_apply_failures: accumulator.failuresByKind.widget_apply,
		data_apply_failures: accumulator.failuresByKind.data_apply,
		page_apply_failures: accumulator.failuresByKind.page_apply,
		tool_failures: accumulator.failuresByKind.tool_error,
		run_failures: accumulator.failuresByKind.run_error,
		failures: boundedFailureSignatures(accumulator),
	};
}

/** Finalize and optionally publish one aggregate-only production metrics payload. */
export function finalizeAgentGenerationMetrics(
	runKey: string,
	outcome: AgentDebugOutcome,
	options: {
		publish?: boolean;
		finalBoardNodeCount?: number;
		/** Terminal reason for a run that failed without any tool event carrying the cause. */
		failure?: { code?: unknown; message?: unknown };
	} = {},
) {
	const accumulator = PRODUCTION_METRICS_RUNS.get(runKey);
	if (!accumulator) return undefined;
	PRODUCTION_METRICS_RUNS.delete(runKey);
	if (
		options.failure &&
		["error", "timeout", "interrupted"].includes(outcome)
	) {
		const message = redactFailureMessage(options.failure.message);
		const code = failureCode(options.failure.code) ?? failureCode(outcome);
		if (message || code) {
			collectFailure(accumulator, `${runKey}:terminal`, {
				kind: "run_error",
				code,
				message,
			});
		}
	}
	if (
		Number.isSafeInteger(options.finalBoardNodeCount) &&
		(options.finalBoardNodeCount ?? -1) >= 0 &&
		accumulator.report.generation_evaluation
	) {
		accumulator.report = {
			...accumulator.report,
			generation_evaluation: {
				...accumulator.report.generation_evaluation,
				final_board_node_count: options.finalBoardNodeCount,
			},
		};
	}
	const metrics = productionMetricsFromAccumulator(accumulator, outcome);
	if (metrics && options.publish !== false) {
		publishFlowPilotProductionMetrics(metrics);
	}
	return metrics;
}

export function clearAgentGenerationMetrics(runKey: string) {
	PRODUCTION_METRICS_RUNS.delete(runKey);
}

export function nestedAgentRunEvent(options: {
	requestId: string;
	parentRequestId: string;
	toolName: string;
	stage: "started" | "finished";
	status?: string;
	input?: unknown;
	output?: unknown;
	error?: unknown;
	summary?: string;
	nowMs?: number;
	/**
	 * Overrides where a failure is attributed. Defaults to the specialist's own domain (a failed
	 * `flowpilot_widget` run is a widget-apply failure); pass `"subagent_dispatch"` when the
	 * specialist never ran, so a transport/dispatch fault is not filed as a build fault.
	 */
	failureKind?: FlowPilotFailureKind;
}): IAgentDebugEvent {
	const now = options.nowMs ?? Date.now();
	const terminal = normalizedTerminalStatus(options.status);
	const started = options.stage === "started";
	const failure = started
		? undefined
		: failureDetailForToolEnd({
				name: options.toolName,
				status: terminal.terminalStatus ?? terminal.status,
				error: options.error,
				rawResult: options.output,
				summary: options.summary,
				fallbackKind: "subagent_dispatch",
				overrideKind: options.failureKind,
			});
	return attachFailureDetail(
		markEventPreviewsNormalized({
			id: `nested:${options.requestId}:run`,
			kind: "nested",
			stage: started ? "nested_run_started" : "nested_run_finished",
			status: started ? "progress" : terminal.status,
			terminal_status: started ? undefined : terminal.terminalStatus,
			name: options.toolName,
			request_id: options.requestId,
			parent_request_id: options.parentRequestId,
			timestamp_ms: now,
			started_at_ms: started ? now : undefined,
			ended_at_ms: started ? undefined : now,
			summary: cleanSummary(options.summary),
			arguments_preview: started
				? agentDebugPreview(options.input, MAX_EVIDENCE_PREVIEW_CHARS)
				: undefined,
			result_preview: started
				? undefined
				: agentDebugPreview(options.output, MAX_EVIDENCE_PREVIEW_CHARS),
			error: started ? undefined : agentDebugPreview(options.error),
		}),
		failure,
	);
}

/**
 * Record the host-side fate of a validated review. Queueing alone must never set `accepted`; only an
 * `applied` disposition does. Pass this event through the same `recordDebugEvent` callback used for
 * tool events so development reports and privacy-safe production counters stay in sync.
 */
export function agentGenerationReviewDispositionEvent(options: {
	requestId: string;
	parentRequestId?: string;
	disposition: AgentGenerationReviewDisposition;
	draftId?: string;
	revision?: number;
	claimId?: string;
	nowMs?: number;
	/**
	 * Why a non-applied disposition happened. Strings are redacted and generalized like every other
	 * failure message; arrays keep only their first usable entry.
	 */
	reason?: { code?: unknown; message?: unknown } | readonly unknown[] | unknown;
}): IAgentDebugEvent {
	const now = options.nowMs ?? Date.now();
	const candidateKey =
		options.draftId && Number.isSafeInteger(options.revision)
			? `${options.draftId}:${String(options.revision)}`
			: undefined;
	const event: GenerationEvidenceEvent = {
		id: `generation:${options.requestId}:review:${options.claimId ?? candidateKey ?? now}`,
		kind: options.parentRequestId ? "nested" : "tool",
		stage: "generation_review_disposition",
		status:
			options.disposition === "applied"
				? "done"
				: options.disposition === "error"
					? "error"
					: "partial",
		terminal_status: options.disposition,
		request_id: options.requestId,
		parent_request_id: options.parentRequestId,
		timestamp_ms: now,
		ended_at_ms: now,
	};
	event[GENERATION_TOOL_EVIDENCE] = {
		kind: "disposition",
		candidateKey,
		disposition: options.disposition,
		accepted: options.disposition === "applied",
	};
	if (options.disposition === "error") {
		const reason = dispositionReason(options.reason);
		attachFailureDetail(event, {
			kind: "flowscript_apply",
			code: reason.code ?? "REVIEW_APPLY_FAILED",
			message: reason.message,
		});
	}
	return markEventPreviewsNormalized(event);
}

/** Accepts the shapes apply paths already hold: a result object, a diagnostics array, or an error. */
function dispositionReason(reason: unknown): {
	code?: string;
	message?: string;
} {
	if (Array.isArray(reason)) {
		for (const entry of reason) {
			const resolved = dispositionReason(entry);
			if (resolved.code || resolved.message) return resolved;
		}
		return {};
	}
	if (reason && typeof reason === "object" && !(reason instanceof Error)) {
		const record = reason as Record<string, unknown>;
		return {
			code: failureCode(record.code ?? record.status),
			message: redactFailureMessage(record.message ?? record.error),
		};
	}
	const message = redactFailureMessage(reason);
	// Diagnostics are conventionally emitted as "CODE: human explanation"; splitting the prefix out
	// makes the trace groupable instead of one bucket per phrasing.
	const prefixed = message?.match(/^([A-Z][A-Z0-9_.:-]{2,}):\s*(.+)$/);
	return prefixed ? { code: prefixed[1], message: prefixed[2] } : { message };
}

export function summarizeAgentDebugRootOutcomes(events: IAgentDebugEvent[]) {
	const rootEvents = events.filter(
		(event) => event.kind !== "nested" && !event.parent_request_id,
	);
	const authoritativeRootStages = new Set([
		"request_completed",
		"request_failed",
		"request_timeout",
		"request_denied",
		"request_cancelled",
		"request_partial",
	]);
	const authoritativeBridgeStages = new Set([
		...authoritativeRootStages,
		"malformed_request",
		"response_delivery_failed",
		"backend_cancelled",
	]);
	const terminalLifecycleStages = new Set([
		"run_finished",
		"stream_error",
		"resume_gap",
	]);
	const latestRootRequest = new Map<string, IAgentDebugEvent>();
	for (const event of rootEvents) {
		if (!event.request_id || !authoritativeRootStages.has(event.stage))
			continue;
		const existing = latestRootRequest.get(event.request_id);
		if (!existing || existing.timestamp_ms <= event.timestamp_ms) {
			latestRootRequest.set(event.request_id, event);
		}
	}
	const relevantRootEvents = rootEvents.filter((event) => {
		if (event.kind === "lifecycle") {
			return terminalLifecycleStages.has(event.stage);
		}
		if (
			event.kind !== "bridge" ||
			!authoritativeBridgeStages.has(event.stage)
		) {
			return false;
		}
		return (
			!event.request_id ||
			!authoritativeRootStages.has(event.stage) ||
			latestRootRequest.get(event.request_id) === event
		);
	});
	const successfulRootRequests = new Set(
		[...latestRootRequest.values()]
			.filter(
				(event) =>
					event.stage === "request_completed" &&
					["done", "ok", "completed"].includes(
						String(event.status ?? "").toLowerCase(),
					),
			)
			.map((event) => event.request_id as string),
	);
	const terminalNestedRuns = new Map<string, IAgentDebugEvent>();
	for (const event of events) {
		if (
			event.kind === "nested" &&
			event.stage === "nested_run_finished" &&
			event.request_id
		) {
			terminalNestedRuns.set(event.request_id, event);
		}
	}
	const relevantNestedTerminals = [...terminalNestedRuns.values()].filter(
		(event) =>
			!event.parent_request_id ||
			!successfulRootRequests.has(event.parent_request_id),
	);
	const relevant = [...relevantRootEvents, ...relevantNestedTerminals];
	// A failed delegation that a LATER root request completed successfully is a recovered
	// attempt, not a failed turn: every delegation carries its own request_id, so the retry can
	// never supersede the failed request's authoritative event by id. Without this, one recovered
	// mid-turn failure marks a fully successful turn as outcome=error forever. Terminal lifecycle
	// failures (stream_error/resume_gap) keep counting — nothing recovers those.
	const lastSuccessfulCompletionMs = Math.max(
		0,
		...[...latestRootRequest.values()]
			.filter(
				(event) =>
					event.stage === "request_completed" &&
					["done", "ok", "completed"].includes(
						String(event.status ?? "").toLowerCase(),
					),
			)
			.map((event) => event.timestamp_ms),
	);
	const recoveredBySuccess = (event: IAgentDebugEvent) =>
		event.kind !== "lifecycle" &&
		lastSuccessfulCompletionMs > 0 &&
		event.timestamp_ms < lastSuccessfulCompletionMs;
	return {
		recordedTimeout: relevant.some(
			(event) =>
				(event.status === "timeout" || event.stage.includes("timeout")) &&
				!recoveredBySuccess(event),
		),
		recordedPartial: relevant.some(
			(event) =>
				(event.status === "partial" ||
					event.status === "denied" ||
					event.status === "cancelled" ||
					event.stage.includes("partial")) &&
				!recoveredBySuccess(event),
		),
		recordedError: relevant.some(
			(event) =>
				(event.status === "error" ||
					event.status === "failed" ||
					Boolean(event.error)) &&
				!recoveredBySuccess(event),
		),
	};
}

function artifactDebugEvent(
	event: CopilotStreamEvent,
	options: {
		scope: "main" | "nested";
		requestId?: string;
		parentRequestId?: string;
		nowMs: number;
		sequence?: number;
	},
): IAgentDebugEvent | null {
	if (
		event.type !== "flowscript_workspace" &&
		event.type !== "components" &&
		event.type !== "commands" &&
		event.type !== "canvas_settings"
	) {
		return null;
	}
	const record = eventRecord(event.data);
	// Live FlowScript snapshots can arrive many times per second while a tool argument is still
	// being written. They are deliberately ephemeral UI state: the submitted/validated/queued
	// snapshot that follows is the durable evidence artifact.
	if (
		event.type === "flowscript_workspace" &&
		String(record.status ?? "").toLowerCase() === "drafting"
	) {
		return null;
	}
	const terminal =
		event.type === "flowscript_workspace"
			? normalizedTerminalStatus(record.status)
			: { status: "done" as const, terminalStatus: undefined };
	const value = event.data ?? event.raw;
	const count = Array.isArray(event.data) ? event.data.length : undefined;
	const source =
		event.type === "flowscript_workspace" && typeof record.source === "string"
			? record.source
			: undefined;
	const summary = source
		? `${source.split(/\r?\n/).length} lines, ${source.length} chars${terminal.terminalStatus ? ` · ${terminal.terminalStatus}` : ""}`
		: count !== undefined
			? `${count} generated item${count === 1 ? "" : "s"}`
			: "Generated artifact";
	return markEventPreviewsNormalized({
		id: `${options.scope}:${options.requestId ?? "run"}:artifact:${event.type}:${options.nowMs}:${options.sequence ?? 0}`,
		kind: options.scope === "nested" ? "nested" : "tool",
		stage: "artifact",
		status: terminal.status,
		terminal_status: terminal.terminalStatus,
		name: event.type,
		request_id: options.requestId,
		parent_request_id: options.parentRequestId,
		timestamp_ms: options.nowMs,
		ended_at_ms: options.nowMs,
		result_summary: summary,
		result_preview: agentDebugPreview(value, MAX_EVIDENCE_PREVIEW_CHARS),
	});
}

/** Convert the shared Copilot stream grammar into chronological report events. */
export function debugEventFromCopilotStream(
	event: CopilotStreamEvent,
	options: {
		scope: "main" | "nested";
		requestId?: string;
		parentRequestId?: string;
		nowMs?: number;
		sequence?: number;
	},
): IAgentDebugEvent | null {
	const now = options.nowMs ?? Date.now();
	const outerRecord = eventRecord(event.data);
	const record =
		event.type === "plan_step" && outerRecord.PlanStep
			? eventRecord(outerRecord.PlanStep)
			: outerRecord;
	const rawId = String(
		record.tool_call_id ?? record.toolCallId ?? record.id ?? "",
	);
	const prefix = `${options.scope}:${options.requestId ?? "run"}`;
	const common = {
		request_id: options.requestId,
		parent_request_id: options.parentRequestId,
		timestamp_ms: now,
	};
	const artifact = artifactDebugEvent(event, {
		...options,
		nowMs: now,
	});
	if (artifact) return artifact;

	// The host emits exactly one structured run summary per terminal run over the tool_end pipe
	// (without a tool_call_id so process-step UIs ignore it). Keep it as a dedicated stage so
	// compaction can pin it and the markdown export can render it as the report headline.
	if (event.type === "tool_end" && record.kind === AGENT_RUN_SUMMARY_STAGE) {
		const summary: Record<string, unknown> = {};
		for (const field of RUN_SUMMARY_FIELDS) {
			if (record[field] !== undefined) summary[field] = record[field];
		}
		const outcome =
			cleanSummary(record.outcome, MAX_GENERATION_DIAGNOSTIC_KEY_CHARS) ??
			"unknown";
		const providerModel = [
			cleanSummary(record.provider, MAX_GENERATION_DIAGNOSTIC_KEY_CHARS),
			cleanSummary(record.model, MAX_GENERATION_DIAGNOSTIC_KEY_CHARS),
		]
			.filter(Boolean)
			.join(" / ");
		return markEventPreviewsNormalized({
			...common,
			id: `${prefix}:run_summary:${now}:${options.sequence ?? 0}`,
			kind: options.scope === "nested" ? "nested" : "lifecycle",
			stage: AGENT_RUN_SUMMARY_STAGE,
			status:
				outcome === "cancelled"
					? "partial"
					: ["committed", "completed"].includes(outcome)
						? "done"
						: "error",
			terminal_status: outcome,
			name: AGENT_RUN_SUMMARY_STAGE,
			result_summary: `${outcome}${providerModel ? ` · ${providerModel}` : ""}${
				typeof record.duration_ms === "number"
					? ` · ${record.duration_ms} ms`
					: ""
			}`,
			result_preview: agentDebugPreview(summary),
			ended_at_ms: now,
		});
	}

	if (event.type === "plan_step") {
		const id = rawId || `plan-${now}`;
		return markEventPreviewsNormalized({
			...common,
			id: `${prefix}:plan:${id}`,
			kind: options.scope === "nested" ? "nested" : "plan",
			stage: "plan",
			status: cleanSummary(record.status) ?? "progress",
			terminal_status: cleanSummary(record.terminal_status),
			name: cleanSummary(record.title ?? record.tool_name),
			summary: cleanSummary(record.description ?? record.message),
			// Only provider-emitted/streamed reasoning is persisted; hidden chain-of-thought is never
			// requested or synthesized by this report layer.
			reasoning: agentDebugPreview(record.reasoning),
			started_at_ms:
				typeof record.start_time === "number" ? record.start_time : undefined,
			ended_at_ms:
				typeof record.end_time === "number" ? record.end_time : undefined,
		});
	}

	if (
		event.type !== "tool_start" &&
		event.type !== "tool_progress" &&
		event.type !== "tool_end"
	)
		return null;
	const id = rawId || `tool-${now}`;
	const name = cleanSummary(
		record.tool_name ?? record.toolName ?? record.tool ?? record.name,
	);
	if (event.type === "tool_start") {
		return markEventPreviewsNormalized({
			...common,
			id: `${prefix}:tool:${id}`,
			kind: options.scope === "nested" ? "nested" : "tool",
			stage: "tool_start",
			status: "progress",
			name,
			summary: cleanSummary(record.summary ?? record.message),
			arguments_preview: agentDebugPreview(
				record.arguments_preview ?? record.arguments ?? record.args,
			),
			started_at_ms: now,
		});
	}
	if (event.type === "tool_progress") {
		return markEventPreviewsNormalized({
			...common,
			id: `${prefix}:tool:${id}`,
			kind: options.scope === "nested" ? "nested" : "tool",
			stage: "tool_progress",
			status: "progress",
			name,
			summary: cleanSummary(record.summary ?? record.message),
		});
	}
	const normalized = normalizedTerminalStatus(
		record.status ?? record.terminal_status,
	);
	const terminalEvent: GenerationEvidenceEvent = {
		...common,
		id: `${prefix}:tool:${id}`,
		kind: options.scope === "nested" ? "nested" : "tool",
		stage: "tool_end",
		status: normalized.status,
		terminal_status:
			cleanSummary(record.terminal_status) ?? normalized.terminalStatus,
		name,
		result_summary: cleanSummary(
			record.result_summary ?? record.summary ?? record.message,
		),
		result_preview: agentDebugPreview(
			record.result_preview ?? record.result ?? record.output,
		),
		error: agentDebugPreview(record.error),
		ended_at_ms: now,
	};
	const evidence = generationEvidenceForToolEnd(
		name,
		record.result_preview ?? record.result ?? record.output,
	);
	if (evidence) terminalEvent[GENERATION_TOOL_EVIDENCE] = evidence;
	return markEventPreviewsNormalized(terminalEvent);
}

function generationMetricEventFromCopilotStream(
	event: CopilotStreamEvent,
	options: {
		scope: "main" | "nested";
		requestId?: string;
		parentRequestId?: string;
		nowMs: number;
		sequence: number;
	},
): IAgentDebugEvent | null {
	if (event.type !== "tool_end") return null;
	const record = eventRecord(event.data);
	const name = cleanSummary(
		record.tool_name ?? record.toolName ?? record.tool ?? record.name,
	);
	const rawResult = record.result_preview ?? record.result ?? record.output;
	const evidence = generationEvidenceForToolEnd(name, rawResult);
	// A failing tool call is worth reporting even when it produced no generation evidence — that is
	// exactly the case where the run stalled and the admin trace needs the reason.
	const isRunSummary = record.kind === AGENT_RUN_SUMMARY_STAGE;
	const failure = failureDetailForToolEnd({
		name,
		status: isRunSummary
			? record.outcome
			: (record.status ?? record.terminal_status),
		error: record.error,
		rawResult,
		summary: record.result_summary ?? record.summary ?? record.message,
		fallbackKind: isRunSummary ? "run_error" : "tool_error",
	});
	if (!evidence && !failure) return null;
	const rawId = String(
		record.tool_call_id ?? record.toolCallId ?? record.id ?? "",
	);
	const metricEvent: GenerationEvidenceEvent = {
		id: `${options.scope}:${options.requestId ?? "run"}:metric:${rawId || options.sequence}`,
		kind: options.scope === "nested" ? "nested" : "tool",
		stage: "tool_end",
		name,
		request_id: options.requestId,
		parent_request_id: options.parentRequestId,
		timestamp_ms: options.nowMs,
		ended_at_ms: options.nowMs,
	};
	if (evidence) metricEvent[GENERATION_TOOL_EVIDENCE] = evidence;
	attachFailureDetail(metricEvent, failure);
	return markEventPreviewsNormalized(metricEvent);
}

export function createAgentDebugStreamRecorder(options: {
	scope: "main" | "nested";
	requestId?: string;
	parentRequestId?: string;
	record: (event: IAgentDebugEvent) => void;
	nowMs?: () => number;
	/** Override only for isolated tests; production callers use the central dev gate. */
	enabled?: boolean;
}) {
	const parser = createCopilotStreamParser();
	const enabled = options.enabled ?? FLOWPILOT_DEBUG_ENABLED;
	let sequence = 0;
	const capture = (events: CopilotStreamEvent[]) => {
		for (const event of events) {
			sequence += 1;
			if (!enabled) {
				const metricEvent = generationMetricEventFromCopilotStream(event, {
					scope: options.scope,
					requestId: options.requestId,
					parentRequestId: options.parentRequestId,
					nowMs: options.nowMs?.() ?? Date.now(),
					sequence,
				});
				if (metricEvent) options.record(metricEvent);
				continue;
			}
			const debugEvent = debugEventFromCopilotStream(event, {
				scope: options.scope,
				requestId: options.requestId,
				parentRequestId: options.parentRequestId,
				nowMs: options.nowMs?.(),
				sequence,
			});
			if (debugEvent) options.record(debugEvent);
		}
		return events;
	};
	return {
		push: (chunk: string) => capture(parser.push(chunk)),
		flush: () => capture(parser.flush()),
	};
}

/** Chronological structured run summaries recorded in a report (unparseable previews are skipped). */
export function agentDebugRunSummaries(
	report: IAgentDebugReport,
): IAgentRunSummary[] {
	return report.events
		.filter(isRunSummaryEvent)
		.sort((left, right) => left.timestamp_ms - right.timestamp_ms)
		.flatMap((event) => {
			if (typeof event.result_preview !== "string") return [];
			try {
				const parsed = JSON.parse(event.result_preview);
				return parsed && typeof parsed === "object" && !Array.isArray(parsed)
					? [parsed as IAgentRunSummary]
					: [];
			} catch {
				return [];
			}
		});
}

function runSummaryDiagnosticCounts(summary: IAgentRunSummary) {
	const counts = new Map<string, number>();
	for (const [code, count] of Object.entries(
		summary.diagnostics_by_code ?? {},
	)) {
		if (typeof count !== "number" || !Number.isFinite(count)) continue;
		counts.set(code, Math.max(0, count));
	}
	return counts;
}

/**
 * Per-code diagnostic trend across multiple run summaries, e.g. `FS_X: 12 -> 3 -> 0 across runs`.
 * A code absent from a run counts as 0 so convergence to zero is visible. Empty below two runs.
 */
export function runSummaryDiagnosticTrends(
	summaries: IAgentRunSummary[],
): string[] {
	if (summaries.length < 2) return [];
	const perRun = summaries.map(runSummaryDiagnosticCounts);
	const codes = [...new Set(perRun.flatMap((counts) => [...counts.keys()]))];
	codes.sort();
	return codes.map((code) => {
		const counts = perRun.map((runCounts) => runCounts.get(code) ?? 0);
		return `${code}: ${counts.join(" -> ")} across runs`;
	});
}

function runSummaryTableLines(summaries: IAgentRunSummary[]) {
	if (summaries.length === 0) return [];
	const budgetCell = (summary: IAgentRunSummary, key: string) => {
		const entry = summary.budget?.[key];
		if (!entry || (entry.used === undefined && entry.limit === undefined)) {
			return "–";
		}
		return `${entry.used ?? 0}/${entry.limit ?? "?"}`;
	};
	const lines = [
		"",
		"## Run summaries",
		"",
		"| # | Outcome | Provider / model | Duration | Phases | Checks | Source ops | Commits | Stalled | Continuations | Diagnostics | Retained draft | Review notes | Applied commands |",
		"| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
	];
	summaries.forEach((summary, index) => {
		const diagnostics = [
			...runSummaryDiagnosticCounts(summary).values(),
		].reduce((total, count) => total + count, 0);
		const draft = summary.retained_draft;
		const draftCell =
			draft && typeof draft === "object" && draft.id
				? `\`${draft.id}@${draft.revision ?? "?"}\``
				: "none";
		lines.push(
			`| ${index + 1} | **${summary.outcome ?? "unknown"}** | ${
				[summary.provider, summary.model].filter(Boolean).join(" / ") ||
				"unknown"
			} | ${summary.duration_ms ?? "?"} ms | ${summary.phases ?? "?"} | ${budgetCell(
				summary,
				"checks",
			)} | ${budgetCell(summary, "source_ops")} | ${budgetCell(summary, "commits")} | ${budgetCell(
				summary,
				"stalled",
			)} | ${budgetCell(summary, "continuations")} | ${diagnostics} | ${draftCell} | ${
				summary.review_notes ?? 0
			} | ${summary.applied_commands ?? 0} |`,
		);
	});
	const trends = runSummaryDiagnosticTrends(summaries);
	if (trends.length > 0) {
		lines.push("", "### Diagnostic trend", "");
		for (const trend of trends) {
			lines.push(`- ${trend}`);
		}
	}
	return lines;
}

export function agentDebugReportAsMarkdown(report: IAgentDebugReport) {
	const lines = [
		"# FlowPilot debug report",
		"",
		`- Schema: \`${report.schema}\``,
		`- Outcome: **${report.outcome}**`,
		`- Duration: ${report.duration_ms ?? "running"} ms`,
		`- Provider/model/effort: ${[report.provider, report.model, report.reasoning_effort].filter(Boolean).join(" / ") || "unknown"}`,
		`- Events: ${report.events.length}${report.truncation ? ` (${report.truncation.events_dropped} dropped)` : ""}`,
	];
	if (report.summary) lines.push(`- Summary: ${report.summary}`);
	if (report.input_preview)
		lines.push(`- Input preview: \`${report.input_preview}\``);
	if (report.output_preview)
		lines.push(`- Output preview: \`${report.output_preview}\``);
	if (report.generation_evaluation) {
		lines.push(
			`- Generation evaluation: ${report.generation_evaluation.attempts.length} attempt(s) · plan ${report.generation_evaluation.plan_outcome} · ${report.generation_evaluation.status}`,
		);
	}
	lines.push(...runSummaryTableLines(agentDebugRunSummaries(report)));
	lines.push("", "## Timeline", "");
	// Tool and nested-run records are updated in place when their terminal frame arrives. Their
	// array position therefore reflects the start frame while timestamp_ms reflects the latest
	// frame. Render a stable chronological copy so a long-running tool_end cannot appear directly
	// after run_started and make the failure chain look out of order.
	const chronologicalEvents = report.events
		.map((event, index) => ({ event, index }))
		.sort(
			(left, right) =>
				left.event.timestamp_ms - right.event.timestamp_ms ||
				left.index - right.index,
		)
		.map(({ event }) => event);
	for (const event of chronologicalEvents) {
		lines.push(
			`- \`${new Date(event.timestamp_ms).toISOString()}\` **${event.stage}**${event.name ? ` · ${event.name}` : ""}${event.status ? ` · ${event.status}` : ""}${event.duration_ms !== undefined ? ` · ${event.duration_ms} ms` : ""}`,
		);
		if (event.request_id || event.parent_request_id) {
			lines.push(
				`  - Correlation:${event.request_id ? ` request=\`${event.request_id}\`` : ""}${event.parent_request_id ? ` parent=\`${event.parent_request_id}\`` : ""}`,
			);
		}
		if (event.terminal_status) {
			lines.push(`  - Terminal status: \`${event.terminal_status}\``);
		}
		if (event.summary) lines.push(`  - ${event.summary}`);
		if (event.arguments_preview)
			lines.push(
				`  - ${event.kind === "nested" ? "Nested input" : "Arguments"}: \`${event.arguments_preview}\``,
			);
		if (event.result_summary) lines.push(`  - Result: ${event.result_summary}`);
		if (event.result_preview)
			lines.push(
				`  - ${event.kind === "nested" ? "Nested output" : "Result preview"}: \`${event.result_preview}\``,
			);
		if (event.reasoning)
			lines.push(`  - Surfaced reasoning: ${event.reasoning}`);
		if (event.error) lines.push(`  - Error: ${event.error}`);
	}
	return lines.join("\n");
}

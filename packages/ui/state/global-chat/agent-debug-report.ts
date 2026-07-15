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
const REPORT_SIZE_CACHE = new WeakMap<IAgentDebugReport, number>();
const GENERATION_CANDIDATE_KEYS = new WeakMap<IAgentDebugReport, string[]>();
const GENERATION_TOOL_EVIDENCE = Symbol("flowpilot-generation-tool-evidence");
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
	| "timeout";

export type AgentDebugEventKind =
	| "lifecycle"
	| "plan"
	| "tool"
	| "approval"
	| "bridge"
	| "nested";

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
 * Privacy-safe production telemetry for workflow generation. Every property after the schema is an
 * aggregate counter. In particular this payload contains no run/message ids, timestamps, model or
 * provider names, prompts, FlowScript, tool arguments/results, board ids, or diagnostic strings.
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

function redactValue(
	value: unknown,
	key = "",
	depth = 0,
	stringLimit = MAX_PREVIEW_CHARS,
	allowWorkspaceEnvelopes = true,
): unknown {
	if (SENSITIVE_KEY.test(key)) return "[REDACTED]";
	if (depth > 5) return "[TRUNCATED_DEPTH]";
	if (typeof value === "string")
		return truncatePreview(
			redactSecretsInText(value, stringLimit, allowWorkspaceEnvelopes),
			stringLimit,
		);
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

function normalizeEvent(event: IAgentDebugEvent): IAgentDebugEvent {
	const startedAt = event.started_at_ms;
	const endedAt = event.ended_at_ms;
	const previewLimit =
		event.stage === "artifact" ||
		(event.kind === "nested" &&
			(event.stage === "nested_run_started" ||
				event.stage === "nested_run_finished"))
			? MAX_EVIDENCE_PREVIEW_CHARS
			: MAX_PREVIEW_CHARS;
	const normalized: IAgentDebugEvent = {
		...event,
		summary: cleanSummary(event.summary),
		arguments_preview: agentDebugPreview(event.arguments_preview, previewLimit),
		result_summary: cleanSummary(event.result_summary),
		result_preview: agentDebugPreview(event.result_preview, previewLimit),
		reasoning: agentDebugPreview(event.reasoning),
		error: cleanSummary(event.error, MAX_PREVIEW_CHARS),
		duration_ms:
			event.duration_ms ??
			(startedAt !== undefined && endedAt !== undefined
				? Math.max(0, endedAt - startedAt)
				: undefined),
	};
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

/** Higher values contain more useful evidence when the report has to shed history. */
function retentionPriority(event: IAgentDebugEvent) {
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
	shrinkPriority(3, 4 * 1024);
	shrinkPriority(3, 1_024);
	dropPriority(3);
	shrinkPriority(4, 8 * 1024);
	shrinkPriority(4, 2 * 1024);
	dropPriority(4);

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
			(event) => !isNestedRunEvidence(event),
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
	const normalized = normalizeEvent(event);
	const index = report.events.findIndex((existing) => existing.id === event.id);
	let reportWithTruncation = report;
	let events: IAgentDebugEvent[];
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
	} else if (report.events.length < MAX_EVENTS) {
		events = [...report.events, normalized];
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
		reportWithTruncation = withReportTruncation(report, {
			events_dropped: (report.truncation?.events_dropped ?? 0) + 1,
			bytes_dropped:
				(report.truncation?.bytes_dropped ?? 0) + valueSize(removed),
		});
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
	const next: IAgentDebugReport = {
		...reportWithTruncation,
		events,
		generation_evaluation: generation.evaluation,
	};
	GENERATION_CANDIDATE_KEYS.set(next, generation.candidateKeys);
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

function parseJsonRecord(value: unknown): Record<string, unknown> | undefined {
	if (value && typeof value === "object" && !Array.isArray(value)) {
		return value as Record<string, unknown>;
	}
	if (typeof value !== "string") return undefined;
	const trimmed = value.trim();
	try {
		const parsed = JSON.parse(trimmed);
		return parsed && typeof parsed === "object" && !Array.isArray(parsed)
			? (parsed as Record<string, unknown>)
			: undefined;
	} catch {
		return undefined;
	}
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
	const raw = normalizedName.endsWith("edit_flowscript");
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
	});
}

/** Consume one already-redacted metric event without retaining the event or its payload. */
export function recordAgentGenerationMetricEvent(
	runKey: string,
	event: IAgentDebugEvent,
) {
	const accumulator = PRODUCTION_METRICS_RUNS.get(runKey);
	if (!accumulator) return;
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
	};
}

/** Finalize and optionally publish one aggregate-only production metrics payload. */
export function finalizeAgentGenerationMetrics(
	runKey: string,
	outcome: AgentDebugOutcome,
	options: { publish?: boolean; finalBoardNodeCount?: number } = {},
) {
	const accumulator = PRODUCTION_METRICS_RUNS.get(runKey);
	if (!accumulator) return undefined;
	PRODUCTION_METRICS_RUNS.delete(runKey);
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
}): IAgentDebugEvent {
	const now = options.nowMs ?? Date.now();
	const terminal = normalizedTerminalStatus(options.status);
	const started = options.stage === "started";
	return {
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
	};
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
	return event;
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
	return {
		recordedTimeout: relevant.some(
			(event) => event.status === "timeout" || event.stage.includes("timeout"),
		),
		recordedPartial: relevant.some(
			(event) =>
				event.status === "partial" ||
				event.status === "denied" ||
				event.status === "cancelled" ||
				event.stage.includes("partial"),
		),
		recordedError: relevant.some(
			(event) =>
				event.status === "error" ||
				event.status === "failed" ||
				Boolean(event.error),
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
	return {
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
	};
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

	if (event.type === "plan_step") {
		const id = rawId || `plan-${now}`;
		return {
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
		};
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
		return {
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
		};
	}
	if (event.type === "tool_progress") {
		return {
			...common,
			id: `${prefix}:tool:${id}`,
			kind: options.scope === "nested" ? "nested" : "tool",
			stage: "tool_progress",
			status: "progress",
			name,
			summary: cleanSummary(record.summary ?? record.message),
		};
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
	return terminalEvent;
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
	const evidence = generationEvidenceForToolEnd(
		name,
		record.result_preview ?? record.result ?? record.output,
	);
	if (!evidence) return null;
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
	metricEvent[GENERATION_TOOL_EVIDENCE] = evidence;
	return metricEvent;
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

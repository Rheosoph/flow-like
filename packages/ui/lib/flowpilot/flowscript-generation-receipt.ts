import type { FlowScriptWorkspaceCandidate } from "../../components/flowpilot/flowscript-workspace-candidates";

export const FLOWSCRIPT_GENERATION_RUN_SCHEMA =
	"flowpilot.flowscript-generation-run/v1" as const;

export type FlowScriptCompilerToolName =
	| "write_flowscript"
	| "patch_flowscript"
	| "check_flowscript"
	| "commit_flowscript"
	| "edit_flowscript";

export interface FlowScriptAuthoredCandidateReceipt
	extends FlowScriptWorkspaceCandidate {
	toolName?: FlowScriptCompilerToolName;
	draftId?: string;
	revision?: number | string;
	capturedAtMs: number;
}

/**
 * Provider-neutral projection of the exact response produced by a FlowScript compiler tool.
 * `payload` retains every structured compiler field, while `source` restores the byte-for-byte
 * authored document carried beside that envelope in `<flowscript_workspace>`.
 */
export interface FlowScriptCompilerReceipt {
	toolName: FlowScriptCompilerToolName;
	status?: string;
	code?: string;
	message?: string;
	draftId?: string;
	revision?: number | string;
	baseFingerprint?: string;
	source?: string;
	diagnostics: readonly unknown[];
	reviewNotes: readonly unknown[];
	corrections: readonly string[];
	derivedCommandCount?: number;
	queuedCount?: number;
	/** True only for a clean terminal result from this specific compiler tool. */
	success: boolean;
	/** Full structured compiler envelope, including fields unknown to this UI version. */
	payload: Readonly<Record<string, unknown>>;
	capturedAtMs: number;
}

export interface FlowScriptGenerationRunReceipt {
	schema: typeof FLOWSCRIPT_GENERATION_RUN_SCHEMA;
	conversationId: string;
	requestId: string;
	parentRequestId?: string;
	appId: string;
	boardId: string;
	provider: string;
	modelId: string;
	reasoningEffort: string;
	startedAtMs: number;
	endedAtMs: number;
	outcome: string;
	candidates: readonly FlowScriptAuthoredCandidateReceipt[];
	compilerReceipts: readonly FlowScriptCompilerReceipt[];
	finalWorkspaceStatus?: string;
	appliedCommands?: number;
	persistedReadbackVerified?: boolean;
}

export interface FlowScriptGenerationTraceMetadata {
	conversationId: string;
	requestId: string;
	parentRequestId?: string;
	appId: string;
	boardId: string;
	provider: string;
	modelId: string;
	reasoningEffort: string;
	startedAtMs?: number;
}

export interface FlowScriptGenerationTraceFinalState {
	outcome: string;
	finalWorkspaceStatus?: string;
	appliedCommands?: number;
	persistedReadbackVerified?: boolean;
	endedAtMs?: number;
}

const TOOL_NAMES: readonly FlowScriptCompilerToolName[] = [
	"write_flowscript",
	"patch_flowscript",
	"check_flowscript",
	"commit_flowscript",
	"edit_flowscript",
];
const MAX_CANDIDATES_PER_RUN = 32;
const MAX_RECEIPTS_PER_RUN = 32;
const MAX_RUNS_PER_CONVERSATION = 32;
const MAX_CONVERSATIONS = 16;
const RUNS_BY_CONVERSATION = new Map<
	string,
	FlowScriptGenerationRunReceipt[]
>();

function record(value: unknown): Record<string, unknown> | undefined {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: undefined;
}

function parseJson(value: string): unknown {
	try {
		return JSON.parse(value);
	} catch {
		return undefined;
	}
}

function toolName(value: unknown): FlowScriptCompilerToolName | undefined {
	const normalized = String(value ?? "")
		.trim()
		.toLowerCase();
	return TOOL_NAMES.find((candidate) => normalized.endsWith(candidate));
}

function resultValue(data: unknown): unknown {
	const container = record(data);
	// Prefer the full result: previews are ellipsis-truncated and corrupt the captured
	// byte-for-byte authored source that compiler-receipt validation depends on.
	return container
		? (container.result ?? container.output ?? container.result_preview)
		: undefined;
}

function taggedValues(value: string, tag: string): unknown[] {
	const values: unknown[] = [];
	const pattern = new RegExp(`<${tag}>([\\s\\S]*?)<\\/${tag}>`, "gi");
	for (const match of value.matchAll(pattern)) {
		const inner = match[1]?.trim();
		if (!inner) continue;
		values.push(parseJson(inner) ?? inner);
	}
	return values;
}

function workspaceCandidate(
	value: unknown,
): FlowScriptWorkspaceCandidate | undefined {
	if (typeof value === "string") {
		const parsed = parseJson(value.trim());
		return parsed === undefined ? undefined : workspaceCandidate(parsed);
	}
	const candidate = record(value);
	if (!candidate || typeof candidate.source !== "string") return undefined;
	const source = candidate.source;
	if (!source.trim()) return undefined;
	return {
		source,
		...(typeof candidate.status === "string"
			? { status: candidate.status }
			: {}),
		...(typeof candidate.completion === "string"
			? { completion: candidate.completion }
			: {}),
		...(typeof candidate.retained_full_source === "string"
			? { retained_full_source: candidate.retained_full_source }
			: {}),
		...(record(candidate.regression)
			? { regression: record(candidate.regression) }
			: {}),
	};
}

function compilerPayloadScore(value: Record<string, unknown>): number {
	let score = 0;
	for (const key of [
		"status",
		"draft_id",
		"revision",
		"base_fingerprint",
		"structured_diagnostics",
		"diagnostics",
		"review_notes",
		"derived_command_count",
		"queued_count",
	]) {
		if (value[key] !== undefined) score += 1;
	}
	return score;
}

interface ParsedCompilerResult {
	payload?: Record<string, unknown>;
	candidates: FlowScriptWorkspaceCandidate[];
	structuredDiagnostics?: unknown[];
}

function parseCompilerResult(value: unknown): ParsedCompilerResult {
	const candidates: FlowScriptWorkspaceCandidate[] = [];
	const payloads: Record<string, unknown>[] = [];
	let structuredDiagnostics: unknown[] | undefined;
	const seen = new WeakSet<object>();

	const visit = (current: unknown, depth: number) => {
		if (depth > 8 || current === null || current === undefined) return;
		if (typeof current === "string") {
			for (const tagged of taggedValues(current, "flowscript_workspace")) {
				const candidate = workspaceCandidate(tagged);
				if (candidate) candidates.push(candidate);
			}
			for (const tag of [
				"flowscript_draft_result",
				"flowscript_commit_result",
			]) {
				for (const tagged of taggedValues(current, tag))
					visit(tagged, depth + 1);
			}
			for (const tagged of taggedValues(current, "structured_diagnostics")) {
				if (Array.isArray(tagged)) structuredDiagnostics = tagged;
			}
			const parsed = parseJson(current.trim());
			if (parsed !== undefined) visit(parsed, depth + 1);
			return;
		}
		if (Array.isArray(current)) {
			for (const entry of current) visit(entry, depth + 1);
			return;
		}
		if (typeof current !== "object" || seen.has(current)) return;
		seen.add(current);
		const container = current as Record<string, unknown>;
		const candidate = workspaceCandidate(container);
		if (candidate) candidates.push(candidate);
		if (compilerPayloadScore(container) >= 2) payloads.push(container);
		if (Array.isArray(container.structured_diagnostics)) {
			structuredDiagnostics = container.structured_diagnostics;
		}
		for (const key of ["text", "content", "result", "output", "data"]) {
			if (container[key] !== undefined) visit(container[key], depth + 1);
		}
	};

	visit(value, 0);
	const payload = payloads
		.sort(
			(left, right) => compilerPayloadScore(left) - compilerPayloadScore(right),
		)
		.at(-1);
	return { payload, candidates, structuredDiagnostics };
}

function stringValue(value: unknown): string | undefined {
	return typeof value === "string" && value.length > 0 ? value : undefined;
}

function revisionValue(value: unknown): number | string | undefined {
	return typeof value === "number" ||
		(typeof value === "string" && value.length > 0)
		? value
		: undefined;
}

function nonNegativeInteger(value: unknown): number | undefined {
	return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
		? value
		: undefined;
}

function compilerSuccess(
	name: FlowScriptCompilerToolName,
	status: string | undefined,
	diagnostics: readonly unknown[],
): boolean {
	if (diagnostics.length > 0) return false;
	const normalized = status?.trim().toLowerCase();
	if (name === "check_flowscript") {
		return normalized === "valid" || normalized === "no_changes";
	}
	if (name === "commit_flowscript" || name === "edit_flowscript") {
		return normalized === "queued" || normalized === "no_changes";
	}
	return false;
}

/** Extract the real compiler envelope from a provider-independent `tool_end` stream event. */
export function extractFlowScriptCompilerReceipt(
	data: unknown,
	latestCandidate?: FlowScriptWorkspaceCandidate,
	nowMs = Date.now(),
): {
	receipt?: FlowScriptCompilerReceipt;
	candidates: FlowScriptWorkspaceCandidate[];
} {
	const container = record(data);
	const name = toolName(
		container?.tool_name ??
			container?.toolName ??
			container?.tool ??
			container?.name,
	);
	if (!name) return { candidates: [] };
	const parsed = parseCompilerResult(resultValue(data));
	const payload = parsed.payload;
	if (!payload) return { candidates: parsed.candidates };
	const candidate = parsed.candidates.at(-1) ?? latestCandidate;
	const status = stringValue(payload.status) ?? candidate?.status;
	const diagnostics = Array.isArray(payload.diagnostics)
		? payload.diagnostics
		: (parsed.structuredDiagnostics ?? []);
	const reviewNotes = Array.isArray(payload.review_notes)
		? payload.review_notes
		: [];
	const corrections = Array.isArray(payload.corrections)
		? payload.corrections.filter(
				(correction): correction is string => typeof correction === "string",
			)
		: [];
	return {
		candidates: parsed.candidates,
		receipt: {
			toolName: name,
			status,
			code: stringValue(payload.code),
			message: stringValue(payload.message),
			draftId: stringValue(payload.draft_id),
			revision: revisionValue(payload.revision),
			baseFingerprint: stringValue(payload.base_fingerprint),
			source:
				stringValue(payload.source) ??
				(candidate?.source.trim() ? candidate.source : undefined),
			diagnostics,
			reviewNotes,
			corrections,
			derivedCommandCount: nonNegativeInteger(payload.derived_command_count),
			queuedCount: nonNegativeInteger(payload.queued_count),
			success: compilerSuccess(name, status, diagnostics),
			payload: { ...payload },
			capturedAtMs: nowMs,
		},
	};
}

export function isSuccessfulFlowScriptCheckReceipt(
	receipt: FlowScriptCompilerReceipt,
): boolean {
	return (
		receipt.toolName === "check_flowscript" &&
		receipt.success &&
		Boolean(receipt.source?.trim())
	);
}

export function isSuccessfulFlowScriptCommitReceipt(
	receipt: FlowScriptCompilerReceipt,
): boolean {
	return (
		(receipt.toolName === "commit_flowscript" ||
			receipt.toolName === "edit_flowscript") &&
		receipt.success &&
		Boolean(receipt.source?.trim())
	);
}

function sameCandidate(
	left: FlowScriptAuthoredCandidateReceipt,
	right: FlowScriptAuthoredCandidateReceipt,
): boolean {
	return (
		left.source === right.source &&
		left.status === right.status &&
		left.completion === right.completion &&
		left.toolName === right.toolName &&
		left.draftId === right.draftId &&
		left.revision === right.revision
	);
}

function publishRun(run: FlowScriptGenerationRunReceipt) {
	const existing = RUNS_BY_CONVERSATION.get(run.conversationId) ?? [];
	const deduped = existing.filter(
		(candidate) => candidate.requestId !== run.requestId,
	);
	RUNS_BY_CONVERSATION.delete(run.conversationId);
	RUNS_BY_CONVERSATION.set(
		run.conversationId,
		[...deduped, run].slice(-MAX_RUNS_PER_CONVERSATION),
	);
	while (RUNS_BY_CONVERSATION.size > MAX_CONVERSATIONS) {
		const oldest = RUNS_BY_CONVERSATION.keys().next().value;
		if (typeof oldest !== "string") break;
		RUNS_BY_CONVERSATION.delete(oldest);
	}
}

/**
 * Turn-local capture for one delegated board compiler run. It is intentionally not persisted:
 * authored FlowScript may contain the same sensitive values as the board document itself.
 */
export function createFlowScriptGenerationTrace(
	metadata: FlowScriptGenerationTraceMetadata,
) {
	const startedAtMs = metadata.startedAtMs ?? Date.now();
	let candidates: FlowScriptAuthoredCandidateReceipt[] = [];
	let compilerReceipts: FlowScriptCompilerReceipt[] = [];
	let finished = false;

	const recordCandidate = (
		candidate: FlowScriptWorkspaceCandidate,
		context: Partial<
			Pick<
				FlowScriptAuthoredCandidateReceipt,
				"toolName" | "draftId" | "revision" | "capturedAtMs"
			>
		> = {},
	) => {
		if (finished || !candidate.source.trim()) return;
		const captured: FlowScriptAuthoredCandidateReceipt = {
			...candidate,
			...context,
			capturedAtMs: context.capturedAtMs ?? Date.now(),
		};
		if (candidates.some((existing) => sameCandidate(existing, captured)))
			return;
		candidates = [...candidates, captured].slice(-MAX_CANDIDATES_PER_RUN);
	};

	return {
		recordCandidate,
		recordToolEnd(data: unknown, nowMs = Date.now()) {
			if (finished) return;
			const extracted = extractFlowScriptCompilerReceipt(
				data,
				candidates.at(-1),
				nowMs,
			);
			for (const candidate of extracted.candidates) {
				recordCandidate(candidate, {
					toolName: extracted.receipt?.toolName,
					draftId: extracted.receipt?.draftId,
					revision: extracted.receipt?.revision,
					capturedAtMs: nowMs,
				});
			}
			if (!extracted.receipt) return;
			compilerReceipts = [...compilerReceipts, extracted.receipt].slice(
				-MAX_RECEIPTS_PER_RUN,
			);
		},
		finish(finalState: FlowScriptGenerationTraceFinalState) {
			if (finished) return undefined;
			finished = true;
			const run: FlowScriptGenerationRunReceipt = {
				schema: FLOWSCRIPT_GENERATION_RUN_SCHEMA,
				conversationId: metadata.conversationId,
				requestId: metadata.requestId,
				parentRequestId: metadata.parentRequestId,
				appId: metadata.appId,
				boardId: metadata.boardId,
				provider: metadata.provider,
				modelId: metadata.modelId,
				reasoningEffort: metadata.reasoningEffort,
				startedAtMs,
				endedAtMs: finalState.endedAtMs ?? Date.now(),
				outcome: finalState.outcome,
				candidates,
				compilerReceipts,
				finalWorkspaceStatus: finalState.finalWorkspaceStatus,
				appliedCommands: finalState.appliedCommands,
				persistedReadbackVerified: finalState.persistedReadbackVerified,
			};
			publishRun(run);
			return run;
		},
	};
}

export function flowScriptGenerationRunsForConversation(
	conversationId: string,
): readonly FlowScriptGenerationRunReceipt[] {
	return RUNS_BY_CONVERSATION.get(conversationId) ?? [];
}

/**
 * Receipts keyed by the app they built, across every conversation. A nested board run inherits its
 * conversation from the tool request and falls back to whichever chat is active, so with several
 * turns in flight a run can be filed under a sibling's conversation. The app is unambiguous.
 */
export function flowScriptGenerationRunsForApp(
	appId: string,
): readonly FlowScriptGenerationRunReceipt[] {
	const runs: FlowScriptGenerationRunReceipt[] = [];
	for (const conversationRuns of RUNS_BY_CONVERSATION.values()) {
		for (const run of conversationRuns) {
			if (run.appId === appId) runs.push(run);
		}
	}
	return runs.sort((left, right) => left.startedAtMs - right.startedAtMs);
}

/** Update a previously published run when an asynchronous native board-edit job settles. */
export function updateFlowScriptGenerationRunReceipt(
	identity: {
		appId: string;
		boardId: string;
		parentRequestId: string;
	},
	patch: Pick<
		FlowScriptGenerationRunReceipt,
		"outcome" | "appliedCommands" | "persistedReadbackVerified"
	> & { endedAtMs?: number },
): FlowScriptGenerationRunReceipt | undefined {
	for (const [conversationId, runs] of RUNS_BY_CONVERSATION) {
		const index = runs.findLastIndex(
			(run) =>
				run.appId === identity.appId &&
				run.boardId === identity.boardId &&
				run.parentRequestId === identity.parentRequestId,
		);
		if (index < 0) continue;
		const current = runs[index];
		if (!current) return undefined;
		const updated: FlowScriptGenerationRunReceipt = {
			...current,
			...patch,
			endedAtMs: patch.endedAtMs ?? Date.now(),
		};
		const next = [...runs];
		next[index] = updated;
		RUNS_BY_CONVERSATION.set(conversationId, next);
		return updated;
	}
	return undefined;
}

export function clearFlowScriptGenerationRuns(conversationId?: string) {
	if (conversationId) RUNS_BY_CONVERSATION.delete(conversationId);
	else RUNS_BY_CONVERSATION.clear();
}

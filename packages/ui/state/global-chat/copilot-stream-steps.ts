import type { CopilotStreamEvent } from "../../components/flowpilot/copilot-stream-parser";
import type {
	IChatUsageStat,
	IPlanStep,
	PlanStepStatus,
} from "../../components/interfaces/chat-default/chat-db";

// Turns the FlowPilot copilot stream protocol (plan_step / tool_start / tool_call / …) into an
// ordered IPlanStep list. Shared by the global chat's own response stream and by nested sub-agent
// runs (e.g. flowpilot_board) whose activity is folded into the same message.

export function mapPlanStepStatus(status: unknown): PlanStepStatus {
	const value = String(status ?? "").toLowerCase();
	if (["done", "completed", "complete", "success"].includes(value))
		return "done";
	if (["failed", "error", "cancelled"].includes(value)) return "failed";
	if (["planned", "pending", "todo", "queued"].includes(value))
		return "planned";
	return "progress";
}

const FAILED_TOOL_RESULT_STATUSES = new Set([
	"error",
	"failed",
	"failure",
	"timeout",
	"timed_out",
	"denied",
	"cancelled",
	"canceled",
	"unresolved",
	"infeasible",
	"zero_progress_circuit_open",
]);

const FAILED_TOOL_RESULT_SUFFIXES = [
	"_error",
	"_errors",
	"_failed",
	"_failure",
	"_timeout",
	"_rejected",
	"_unavailable",
	"_violation",
	"_mismatch",
	"_conflict",
	"_exhausted",
	"_stalled",
	"_blocked",
	"_refused",
	"_needs_repair",
] as const;

const ADVISORY_TOOL_RESULT_STATUSES = new Set([
	"validation_error",
	"validation_errors",
	"draft_needs_repair",
	"module_needs_repair",
	"scope_plan_accepted",
	"scope_plan_required",
	"declaration_lookup_required",
	"declaration_lookup_in_flight",
	"declaration_batch_required",
	"declaration_follow_up_unrelated",
	"diagnostic_lookup_required",
	"duplicate_declaration_lookup",
	"retained_revision_required",
	"flowscript_draft_required",
	"commit_validated_prefix",
	"predraft_inspection_budget_exhausted",
	"discovery_budget_exhausted",
	"time_budget_extended",
	"time_budget_unavailable",
	"deferred",
]);

function explicitToolResultError(
	record: Record<string, unknown>,
): boolean | undefined {
	const value = record.is_error ?? record.isError;
	return typeof value === "boolean" ? value : undefined;
}

/**
 * Settle a completed tool call without maintaining a provider-status allowlist.
 *
 * Explicit `is_error` metadata is authoritative. Without it, only known-negative status names
 * fail; accepted plans, host redirects/advisories, and future successful status names settle as
 * done instead of becoming false failures in the activity timeline.
 */
export function toolEndPlanStepStatus(data: unknown): "done" | "failed" {
	const record =
		data && typeof data === "object"
			? (data as Record<string, unknown>)
			: { status: data };
	const explicitError = explicitToolResultError(record);
	if (explicitError !== undefined) return explicitError ? "failed" : "done";

	const status = String(record.status ?? record.terminal_status ?? "")
		.trim()
		.toLowerCase();
	if (ADVISORY_TOOL_RESULT_STATUSES.has(status)) return "done";
	if (
		FAILED_TOOL_RESULT_STATUSES.has(status) ||
		FAILED_TOOL_RESULT_SUFFIXES.some((suffix) => status.endsWith(suffix))
	) {
		return "failed";
	}
	return "done";
}

export function readPlanStep(
	data: unknown,
): (Omit<IPlanStep, "timestamp"> & { toolName?: string }) | null {
	if (!data || typeof data !== "object") return null;
	const source = (data as { PlanStep?: unknown }).PlanStep ?? data;
	if (!source || typeof source !== "object") return null;
	const record = source as Record<string, unknown>;
	const id = String(record.id ?? record.step_id ?? "");
	if (!id) return null;
	const toolName =
		typeof record.tool_name === "string" ? record.tool_name : undefined;
	const title = String(
		record.title ?? record.tool_name ?? record.message ?? "Step",
	);
	const description =
		typeof record.description === "string"
			? record.description
			: typeof record.message === "string"
				? record.message
				: undefined;
	const reasoning =
		typeof record.reasoning === "string" ? record.reasoning : undefined;
	const status = mapPlanStepStatus(record.status);
	// "think" steps carry the model's whole reasoning as description — surface it behind the
	// expandable reasoning viewer instead of an always-visible wall of text.
	if (toolName === "think") {
		return {
			id,
			title: "Thinking",
			reasoning: reasoning ?? description,
			status,
			toolName,
		};
	}
	return { id, title, description, status, reasoning, toolName };
}

export interface StreamAccumulator {
	content: string;
	stepOrder: string[];
	steps: Map<string, IPlanStep>;
	currentStepId?: string;
	/** Usage/stats frames (`<usage_stat>`) the agent streamed for its own model calls. */
	usageStats: IChatUsageStat[];
}

export function createStreamAccumulator(): StreamAccumulator {
	return { content: "", stepOrder: [], steps: new Map(), usageStats: [] };
}

/** The accumulator's steps in stream order. */
export function orderedSteps(acc: StreamAccumulator): IPlanStep[] {
	return acc.stepOrder
		.map((id) => acc.steps.get(id))
		.filter((step): step is IPlanStep => step !== undefined);
}

/** Concatenate usage-stat lists, dropping exact duplicates (same JSON) across sources. */
export function mergeUsageStats(
	...lists: IChatUsageStat[][]
): IChatUsageStat[] {
	const seen = new Set<string>();
	const merged: IChatUsageStat[] = [];
	for (const list of lists) {
		for (const stat of list) {
			const signature = JSON.stringify(stat);
			if (seen.has(signature)) continue;
			seen.add(signature);
			merged.push(stat);
		}
	}
	return merged;
}

/** Narrow an arbitrary stream payload to an IChatUsageStat, or null if it isn't one. */
export function readUsageStat(data: unknown): IChatUsageStat | null {
	if (!data || typeof data !== "object") return null;
	const record = data as Record<string, unknown>;
	const stats = record.stats;
	if (!stats || typeof stats !== "object") return null;
	if (!("usage" in (stats as Record<string, unknown>))) return null;
	return {
		step_name:
			typeof record.step_name === "string" ? record.step_name : "Assistant",
		stats: stats as IChatUsageStat["stats"],
	};
}

function toolFieldId(data: unknown, fallback: string): string {
	const record = (data ?? {}) as Record<string, unknown>;
	return String(
		record.tool_call_id ?? record.toolCallId ?? record.id ?? fallback,
	);
}

function toolFieldName(data: unknown): string {
	const record = (data ?? {}) as Record<string, unknown>;
	// The SDK/external backends put the name under `tool`; the rig path uses `tool_name`.
	return String(
		record.tool_name ?? record.toolName ?? record.tool ?? record.name ?? "tool",
	);
}

function toolFieldSummary(data: unknown): string | undefined {
	const record = (data ?? {}) as Record<string, unknown>;
	const summary = record.summary ?? record.message;
	return typeof summary === "string" && summary.trim() ? summary : undefined;
}

export function applyStreamEvent(
	acc: StreamAccumulator,
	event: CopilotStreamEvent,
) {
	const upsertStep = (step: IPlanStep) => {
		if (!acc.steps.has(step.id)) acc.stepOrder.push(step.id);
		acc.steps.set(step.id, step);
	};

	switch (event.type) {
		case "text":
			if (event.text) acc.content += event.text;
			break;
		case "plan_step": {
			const parsed = readPlanStep(event.data);
			if (!parsed) break;
			const { toolName, ...step } = parsed;
			// Drop redundant "Running X"/"Ran X" descriptions so the merged entry keeps the
			// meaningful text and later `?? existing` merges don't resurrect boilerplate.
			if (
				toolName &&
				(step.description === `Running ${toolName}` ||
					step.description === `Ran ${toolName}`)
			) {
				step.description = undefined;
			}
			const existing = acc.steps.get(step.id);
			upsertStep({
				...step,
				description: step.description ?? existing?.description,
				reasoning: step.reasoning ?? existing?.reasoning,
				// Kept on the step so the orb can tell research from code generation.
				toolName: toolName ?? existing?.toolName,
				timestamp: existing?.timestamp ?? Date.now(),
				content_offset: existing?.content_offset ?? acc.content.length,
			});
			acc.currentStepId =
				step.status === "progress" || step.status === "planned"
					? step.id
					: undefined;
			break;
		}
		case "tool_start": {
			const id = toolFieldId(event.data, `tool-${acc.stepOrder.length}`);
			const name = toolFieldName(event.data);
			const existing = acc.steps.get(id);
			upsertStep({
				id,
				title: `Using ${name}`,
				description: toolFieldSummary(event.data),
				status: "progress",
				toolName: name,
				timestamp: existing?.timestamp ?? Date.now(),
				content_offset: existing?.content_offset ?? acc.content.length,
			});
			acc.currentStepId = id;
			break;
		}
		case "tool_progress": {
			const id = toolFieldId(event.data, `progress-${acc.stepOrder.length}`);
			const existing = acc.steps.get(id);
			const message = toolFieldSummary(event.data);
			if (existing) {
				if (message) acc.steps.set(id, { ...existing, description: message });
				break;
			}
			// External backends (codex/claude-code, some copilot phases) emit tool_progress
			// WITHOUT a preceding tool_start — create the step or their activity is invisible.
			const name = toolFieldName(event.data);
			upsertStep({
				id,
				title: name === "tool" ? "Working" : `Using ${name}`,
				description: message,
				status: "progress",
				timestamp: Date.now(),
				content_offset: acc.content.length,
			});
			acc.currentStepId = id;
			break;
		}
		case "tool_end": {
			const id = toolFieldId(event.data, "");
			const existing = id ? acc.steps.get(id) : undefined;
			if (existing) {
				acc.steps.set(id, {
					...existing,
					status: toolEndPlanStepStatus(event.data),
				});
			}
			acc.currentStepId = undefined;
			break;
		}
		case "usage_stat": {
			const stat = readUsageStat(event.data);
			if (!stat) break;
			// Dedup by identity: a backend may re-emit the same aggregated stat frame.
			const signature = JSON.stringify(stat);
			if (!acc.usageStats.some((s) => JSON.stringify(s) === signature)) {
				acc.usageStats.push(stat);
			}
			break;
		}
		default:
			break;
	}
}

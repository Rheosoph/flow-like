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
			const record = (event.data ?? {}) as Record<string, unknown>;
			if (existing) {
				const terminalStatus = String(
					record.status ?? record.terminal_status ?? "done",
				).toLowerCase();
				const failed = [
					"error",
					"failed",
					"failure",
					"timeout",
					"timed_out",
					"cancelled",
					"canceled",
					"denied",
					"validation_error",
					"validation_errors",
				].includes(terminalStatus);
				acc.steps.set(id, {
					...existing,
					status: failed ? "failed" : "done",
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

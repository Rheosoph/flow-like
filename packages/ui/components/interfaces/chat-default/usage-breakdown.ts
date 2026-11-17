import type {
	IChatUsageStat,
	IModelCallEntry,
	IModelUsage,
} from "../../../lib/llm/usage";

export type UsagePurpose = "context_summary" | "memory" | "vision";

export interface UsageCallRow {
	model: string;
	purpose: UsagePurpose | null;
	promptTokens: number;
	completionTokens: number;
	totalTokens: number;
	/** Input read from the provider cache; null when the call did not report it. */
	cachedTokens: number | null;
	cost: number | null;
	durationMs: number | null;
}

export interface UsageStepRow {
	name: string;
	iterations: number | null;
	calls: UsageCallRow[];
	totalTokens: number;
	cost: number | null;
	durationMs: number | null;
}

export interface UsageModelRow {
	model: string;
	calls: number;
	totalTokens: number;
	promptTokens: number;
	completionTokens: number;
	cost: number | null;
	/** Part of this reply's tokens, 0–1. */
	share: number;
}

export interface UsageBreakdown {
	totalTokens: number;
	promptTokens: number;
	completionTokens: number;
	cost: number | null;
	durationMs: number | null;
	/** Listed calls plus earlier calls that only count toward the totals. */
	callCount: number;
	models: UsageModelRow[];
	steps: UsageStepRow[];
	/** Context summary calls, which run on top of the answer itself. */
	summary: { totalTokens: number; cost: number | null } | null;
	/** Models with calls that left cache reads unreported, and their input tokens. */
	cacheUnreported: { models: string[]; promptTokens: number };
}

export function usagePurpose(purpose: string | null | undefined) {
	if (purpose === "context_compaction") return "context_summary";
	if (purpose?.startsWith("memory_")) return "memory";
	if (purpose === "vision") return "vision";
	return null;
}

function addCost(sum: number | null, cost: number | null | undefined) {
	return cost == null ? sum : (sum ?? 0) + cost;
}

function callRow(
	model: string,
	usage: IModelUsage,
	durationMs: number | null | undefined,
	purpose: string | null | undefined,
): UsageCallRow {
	return {
		model,
		purpose: usagePurpose(purpose),
		promptTokens: usage.prompt_tokens,
		completionTokens: usage.completion_tokens,
		totalTokens: usage.total_tokens,
		cachedTokens: usage.prompt_tokens_details?.cached_tokens ?? null,
		cost: usage.cost ?? null,
		durationMs: durationMs ?? null,
	};
}

function stepCalls(stat: IChatUsageStat): UsageCallRow[] {
	const calls: IModelCallEntry[] = stat.stats.calls ?? [];
	if (calls.length > 0)
		return calls.map((call) =>
			callRow(
				call.model || "unknown",
				call.usage,
				call.duration_ms,
				call.purpose,
			),
		);
	if (!stat.stats.model) return [];
	return [
		callRow(stat.stats.model, stat.stats.usage, stat.stats.duration_ms, null),
	];
}

/** Totals, per-model and per-step rows for one reply's parsed usage stats. */
export function usageBreakdown(
	stats: readonly IChatUsageStat[],
): UsageBreakdown {
	let promptTokens = 0;
	let completionTokens = 0;
	let totalTokens = 0;
	let cost: number | null = null;
	let durationMs: number | null = null;
	let callCount = 0;
	let summary: UsageBreakdown["summary"] = null;
	const unreported = new Set<string>();
	let unreportedPrompt = 0;
	const byModel = new Map<string, UsageModelRow>();

	const steps = stats.map((stat): UsageStepRow => {
		const usage = stat.stats.usage;
		promptTokens += usage.prompt_tokens;
		completionTokens += usage.completion_tokens;
		totalTokens += usage.total_tokens;
		cost = addCost(cost, usage.cost);
		if (stat.stats.duration_ms != null)
			durationMs = (durationMs ?? 0) + stat.stats.duration_ms;
		const calls = stepCalls(stat);
		callCount += Math.max(calls.length, 1) + (stat.stats.calls_omitted ?? 0);
		for (const call of calls) {
			const row = byModel.get(call.model) ?? {
				model: call.model,
				calls: 0,
				totalTokens: 0,
				promptTokens: 0,
				completionTokens: 0,
				cost: null,
				share: 0,
			};
			row.calls += 1;
			row.totalTokens += call.totalTokens;
			row.promptTokens += call.promptTokens;
			row.completionTokens += call.completionTokens;
			row.cost = addCost(row.cost, call.cost);
			byModel.set(call.model, row);
			if (call.purpose === "context_summary")
				summary = {
					totalTokens: (summary?.totalTokens ?? 0) + call.totalTokens,
					cost: addCost(summary?.cost ?? null, call.cost),
				};
			if (call.cachedTokens === null && call.promptTokens > 0) {
				unreported.add(call.model);
				unreportedPrompt += call.promptTokens;
			}
		}
		return {
			name: stat.step_name,
			iterations: stat.stats.iterations ?? null,
			calls,
			totalTokens: usage.total_tokens,
			cost:
				usage.cost ??
				calls.reduce<number | null>(
					(sum, call) => addCost(sum, call.cost),
					null,
				),
			durationMs: stat.stats.duration_ms ?? null,
		};
	});

	const modelTotal = Math.max(
		totalTokens,
		Array.from(byModel.values()).reduce((sum, row) => sum + row.totalTokens, 0),
		1,
	);
	const models = Array.from(byModel.values())
		.map((row) => ({ ...row, share: row.totalTokens / modelTotal }))
		.sort((a, b) => b.totalTokens - a.totalTokens);

	return {
		totalTokens,
		promptTokens,
		completionTokens,
		cost,
		durationMs,
		callCount,
		models,
		steps,
		summary,
		cacheUnreported: {
			models: models
				.map((row) => row.model)
				.filter((model) => unreported.has(model)),
			promptTokens: unreportedPrompt,
		},
	};
}

function compact(value: number) {
	return value >= 100
		? String(Math.round(value))
		: value.toFixed(1).replace(/\.0$/, "");
}

export function formatTokens(count: number): string {
	if (count >= 1_000_000) return `${compact(count / 1_000_000)}M`;
	if (count >= 1_000) return `${compact(count / 1_000)}k`;
	return String(Math.round(count));
}

export function formatCost(cost: number): string {
	if (cost < 0.01) return `$${cost.toFixed(4)}`;
	if (cost < 1) return `$${cost.toFixed(3)}`;
	return `$${cost.toFixed(2)}`;
}

export function formatDuration(ms: number): string {
	if (ms < 1000) return `${Math.round(ms)} ms`;
	return `${(ms / 1000).toFixed(1)} s`;
}

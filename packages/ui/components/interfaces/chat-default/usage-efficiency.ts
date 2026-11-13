import {
	type IChatUsageStat,
	type ICompactionUsage,
	type IContextUsage,
	type IModelUsage,
	readUsageStat,
} from "../../../lib/llm/usage";

function count(value: unknown): number | null {
	return typeof value === "number" && Number.isFinite(value) && value >= 0
		? value
		: null;
}

export interface UsageEfficiency {
	cacheReadTokens: number | null;
	cacheWriteTokens: number | null;
	reportedInputTokens: number;
	inputTokens: number;
	cacheShare: number | null;
	partialReads: boolean;
	partialWrites: boolean;
	context:
		| (IContextUsage & { step: string; tokens: number; measured: boolean })
		| null;
	compactions: (ICompactionUsage & { step: string })[];
	maintenanceTokens: number | null;
	maintenanceCost: number | null;
	hasMemoryWork: boolean;
	callsOmitted: number;
	compactionsOmitted: number;
}

/** Use only input whose cache reads were reported as the cache-share denominator. */
export function usageEfficiency(
	stats: readonly IChatUsageStat[],
): UsageEfficiency {
	let reads: number | null = null;
	let writes: number | null = null;
	let input = 0;
	let reportedInput = 0;
	let reportedReads = 0;
	let missingReads = false;
	let missingWrites = false;
	let context: UsageEfficiency["context"] = null;
	const compactions: UsageEfficiency["compactions"] = [];
	let maintenanceTokens: number | null = null;
	let maintenanceCost: number | null = null;
	let hasMemoryWork = false;
	let callsOmitted = 0;
	let compactionsOmitted = 0;
	for (const raw of stats) {
		const parsed = readUsageStat(raw);
		if (!parsed) continue;
		const { step_name: step, stats: stat } = parsed;
		callsOmitted += count(stat.calls_omitted) ?? 0;
		compactionsOmitted += count(stat.compactions_omitted) ?? 0;
		input += count(stat.usage.prompt_tokens) ?? 0;
		if ((stat.calls_omitted ?? 0) > 0) {
			missingReads = true;
			missingWrites = true;
		}
		const calls = Array.isArray(stat.calls) ? stat.calls : [];
		if (
			calls.length > 0 &&
			calls.reduce((sum, call) => sum + call.usage.prompt_tokens, 0) <
				stat.usage.prompt_tokens
		) {
			missingReads = true;
			missingWrites = true;
		}
		const usages: IModelUsage[] = calls.length
			? calls.map((call) => call.usage)
			: [stat.usage];
		let currentReads: number | null = null;
		let currentWrites: number | null = null;
		const coverageKnown = calls.length > 0 || !(stat.calls_omitted ?? 0);
		for (const usage of usages) {
			const prompt = count(usage?.prompt_tokens) ?? 0;
			const read = count(usage?.prompt_tokens_details?.cached_tokens);
			const write = count(usage?.prompt_tokens_details?.cache_write_tokens);
			if (read !== null) {
				currentReads = (currentReads ?? 0) + Math.min(read, prompt);
				if (coverageKnown) {
					reportedInput += prompt;
					reportedReads += Math.min(read, prompt);
				}
			} else if (prompt > 0) missingReads = true;
			if (write !== null)
				currentWrites = (currentWrites ?? 0) + Math.min(write, prompt);
			else if (prompt > 0) missingWrites = true;
		}
		const totalReads =
			count(stat.usage.prompt_tokens_details?.cached_tokens) ?? currentReads;
		const totalWrites =
			count(stat.usage.prompt_tokens_details?.cache_write_tokens) ??
			currentWrites;
		if (totalReads !== null)
			reads = (reads ?? 0) + Math.min(totalReads, stat.usage.prompt_tokens);
		if (totalWrites !== null)
			writes = (writes ?? 0) + Math.min(totalWrites, stat.usage.prompt_tokens);
		const current = stat.context;
		if (
			current &&
			(count(current.context_window) ?? 0) > 0 &&
			(context?.step !== "Assistant" || step === "Assistant")
		) {
			const measured = count(current.measured_tokens);
			const tokens = measured ?? count(current.estimated_tokens);
			if (tokens !== null)
				context = { ...current, step, tokens, measured: measured !== null };
		}
		for (const event of Array.isArray(stat.compactions)
			? stat.compactions
			: []) {
			if (event && count(event.before_tokens) !== null)
				compactions.push({ ...event, step });
		}
		const maintenanceCalls = calls.filter(
			(call) =>
				call.purpose === "context_compaction" ||
				call.purpose?.startsWith("memory_"),
		);
		hasMemoryWork ||=
			maintenanceCalls.some((call) => call.purpose?.startsWith("memory_")) ||
			(stat.compactions ?? []).some((event) => event.kind === "memory");
		// Compaction events describe calls already included in totals. Never add them twice.
		const maintenanceUsage = maintenanceCalls.length
			? maintenanceCalls.map((call) => call.usage)
			: (stat.compactions ?? []).flatMap((event) =>
					event.usage ? [event.usage] : [],
				);
		for (const usage of maintenanceUsage) {
			const tokens = count(usage.total_tokens);
			const cost = count(usage.cost);
			if (tokens !== null)
				maintenanceTokens = (maintenanceTokens ?? 0) + tokens;
			if (cost !== null) maintenanceCost = (maintenanceCost ?? 0) + cost;
		}
	}
	return {
		cacheReadTokens: reads,
		cacheWriteTokens: writes,
		reportedInputTokens: reportedInput,
		inputTokens: input,
		cacheShare:
			reads !== null && reportedInput > 0
				? reportedReads / reportedInput
				: null,
		partialReads: reads !== null && missingReads,
		partialWrites: writes !== null && missingWrites,
		context,
		compactions,
		maintenanceTokens,
		maintenanceCost,
		hasMemoryWork,
		callsOmitted,
		compactionsOmitted,
	};
}

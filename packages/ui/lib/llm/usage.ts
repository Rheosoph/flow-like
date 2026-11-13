/** Provider counts. Missing cache fields mean the provider did not report them. */
export interface IModelUsage {
	completion_tokens: number;
	prompt_tokens: number;
	total_tokens: number;
	cost?: number | null;
	prompt_tokens_details?: {
		cached_tokens?: number | null;
		cache_write_tokens?: number | null;
		audio_tokens?: number | null;
	} | null;
}

export interface IModelCallEntry {
	model: string;
	usage: IModelUsage;
	duration_ms?: number | null;
	purpose?: string | null;
}

export interface IContextUsage {
	estimated_tokens: number;
	measured_tokens?: number | null;
	context_window: number;
	reserved_output_tokens: number;
}

export interface ICompactionUsage {
	kind: string;
	trigger: string;
	status: string;
	before_tokens: number;
	after_tokens?: number | null;
	duration_ms: number;
	model?: string | null;
	usage?: IModelUsage | null;
}

export interface ILLMUsageStats {
	usage: IModelUsage;
	model?: string | null;
	duration_ms?: number | null;
	iterations?: number | null;
	calls?: IModelCallEntry[];
	calls_omitted?: number;
	context?: IContextUsage | null;
	compactions?: ICompactionUsage[];
	compactions_omitted?: number;
}

export interface IChatUsageStat {
	step_name: string;
	/** Identifies a nested stream so replayed snapshots replace only that source. */
	source_id?: string;
	stats: ILLMUsageStats;
}

/** Replace scoped snapshots while keeping independent sources with equal usage. */
export function mergeUsageStats(
	...lists: IChatUsageStat[][]
): IChatUsageStat[] {
	const positions = new Map<string, number>();
	const merged: IChatUsageStat[] = [];
	for (const stat of lists.flat()) {
		const signature = JSON.stringify(stat);
		const key = stat.source_id
			? JSON.stringify([stat.source_id, stat.step_name])
			: signature;
		const index = positions.get(key);
		if (index === undefined) {
			positions.set(key, merged.length);
			merged.push(stat);
		} else if (JSON.stringify(merged[index]) !== signature)
			merged[index] = stat;
	}
	return merged;
}

function record(value: unknown): value is Record<string, unknown> {
	return value !== null && typeof value === "object" && !Array.isArray(value);
}

function nonNegative(value: unknown): value is number {
	return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function cleanOptional(
	value: Record<string, unknown>,
	numbers: string[],
	strings: string[] = [],
) {
	for (const key of numbers) {
		if (value[key] != null && !nonNegative(value[key])) delete value[key];
	}
	for (const key of strings) {
		if (value[key] != null && typeof value[key] !== "string") delete value[key];
	}
}

function modelUsage(value: unknown): IModelUsage | null {
	if (
		!record(value) ||
		!["prompt_tokens", "completion_tokens", "total_tokens"].every((key) =>
			nonNegative(value[key]),
		)
	)
		return null;
	const usage = { ...value };
	cleanOptional(usage, ["cost"]);
	if (record(usage.prompt_tokens_details)) {
		const details = { ...usage.prompt_tokens_details };
		cleanOptional(details, [
			"cached_tokens",
			"cache_write_tokens",
			"audio_tokens",
		]);
		usage.prompt_tokens_details = details;
	} else if (usage.prompt_tokens_details != null)
		usage.prompt_tokens_details = undefined;
	return usage as unknown as IModelUsage;
}

/** Preserve additive metadata, dropping malformed optional details from old or remote replies. */
export function readUsageStat(data: unknown): IChatUsageStat | null {
	if (!record(data) || !record(data.stats)) return null;
	const usage = modelUsage(data.stats.usage);
	if (!usage) return null;
	const stats: Record<string, unknown> = { ...data.stats, usage };
	cleanOptional(
		stats,
		["duration_ms", "iterations", "calls_omitted", "compactions_omitted"],
		["model"],
	);
	if (Array.isArray(stats.calls)) {
		stats.calls = stats.calls.flatMap((value) => {
			if (!record(value)) return [];
			const usage = modelUsage(value.usage);
			if (!usage) return [];
			const call = {
				...value,
				usage,
				model: typeof value.model === "string" ? value.model : "",
			};
			cleanOptional(call, ["duration_ms"], ["purpose"]);
			return [call];
		});
	} else if ("calls" in stats) stats.calls = undefined;
	if (
		record(stats.context) &&
		["estimated_tokens", "context_window", "reserved_output_tokens"].every(
			(key) => nonNegative((stats.context as Record<string, unknown>)[key]),
		)
	) {
		stats.context = { ...stats.context };
		cleanOptional(stats.context as Record<string, unknown>, [
			"measured_tokens",
		]);
	} else if ("context" in stats) stats.context = undefined;
	if (Array.isArray(stats.compactions)) {
		stats.compactions = stats.compactions.flatMap((value) => {
			if (
				!record(value) ||
				!["kind", "trigger", "status"].every(
					(key) => typeof value[key] === "string",
				) ||
				!nonNegative(value.before_tokens) ||
				!nonNegative(value.duration_ms)
			)
				return [];
			const event = { ...value };
			cleanOptional(event, ["after_tokens"], ["model"]);
			if (event.usage != null) {
				const usage = modelUsage(event.usage);
				if (usage) event.usage = usage;
				else event.usage = undefined;
			}
			return [event];
		});
	} else if ("compactions" in stats) stats.compactions = undefined;
	return {
		step_name:
			typeof data.step_name === "string" ? data.step_name : "Assistant",
		...(typeof data.source_id === "string"
			? { source_id: data.source_id }
			: {}),
		stats: stats as unknown as ILLMUsageStats,
	};
}

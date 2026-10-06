import type {
	HostedModel,
	ModelBackend,
	ModelEngine,
	ModelJob,
	ModelSettings,
	ModelStats,
	ModelsOverview,
	ModelsSummary,
	Recommendation,
	Residency,
	RuntimeInfo,
	StatsStep,
} from "../../../../lib/device-management/models";
import { STATS_STEPS } from "../../../../lib/device-management/models";
import type { SeverityKind } from "../primitives/icons";

/*
 * What the Models tab derives from the `models` answers (plan §3.7): pure, so
 * the copy layer only maps codes and numbers to sentences.
 */

export const TIER_SEVERITY: Record<Recommendation["tier"], SeverityKind> = {
	now: "critical",
	soon: "warning",
	later: "notice",
};

/** Recommendations the device files under "now": they badge the tab. */
export function nowCount(recommendations: readonly Recommendation[]): number {
	return recommendations.filter((item) => item.tier === "now").length;
}

export type HeadlineLead =
	| { kind: "empty" }
	| { kind: "failed"; failed: number }
	| { kind: "serving"; loaded: number; models: number }
	| { kind: "idle"; models: number };

export interface ModelsHeadline {
	lead: HeadlineLead;
	tokens24h: number;
	requests24h: number;
	errors24h: number;
	/** Used share of the GPU memory, 0–100; absent without a GPU that reports it. */
	gpuMemoryPercent?: number;
	/** Models still downloading their files. */
	downloading: number;
}

const percentOf = (used: number, total: number) =>
	total > 0 ? Math.round((used / total) * 100) : undefined;

/** The busiest GPU that reports its memory. */
export function gpuMemoryPercent(
	system: ModelsOverview["system"],
): number | undefined {
	const shares = system.gpus.flatMap((gpu) =>
		gpu.memory_total === null || gpu.memory_free === null
			? []
			: [percentOf(gpu.memory_total - gpu.memory_free, gpu.memory_total)],
	);
	const known = shares.filter((share): share is number => share !== undefined);
	return known.length ? Math.max(...known) : undefined;
}

/** The conclusion and the day's traffic from the device's counters alone, as the encrypted metrics snapshot carries them. */
export function summaryHeadline(summary: ModelsSummary): ModelsHeadline {
	const lead: HeadlineLead = !summary.models
		? { kind: "empty" }
		: summary.failed
			? { kind: "failed", failed: summary.failed }
			: summary.loaded
				? { kind: "serving", loaded: summary.loaded, models: summary.models }
				: { kind: "idle", models: summary.models };
	return {
		lead,
		tokens24h: summary.tokens_24h,
		requests24h: summary.requests_24h,
		errors24h: summary.errors_24h,
		downloading: 0,
	};
}

/** SPEC §4.24: one conclusion, then the day's traffic. */
export function modelsHeadline(overview: ModelsOverview): ModelsHeadline {
	const downloading = overview.models.filter(
		(model) => model.state === "acquiring",
	).length;
	const gpu = gpuMemoryPercent(overview.system);
	return {
		...summaryHeadline(overview.summary),
		downloading,
		...(gpu === undefined ? {} : { gpuMemoryPercent: gpu }),
	};
}

const GPU_BACKENDS: readonly ModelBackend[] = ["cuda", "vulkan", "metal"];

/** The backend an engine runs on: the installed GPU runtime when there is one. */
export function backendOf(
	engine: ModelEngine,
	runtimes: readonly RuntimeInfo[],
): ModelBackend | undefined {
	if (engine === "onnx") return "cpu";
	if (engine === "mlx") return "metal";
	const installed = runtimes.filter(
		(row) => row.runtime === engine && row.installed,
	);
	return (
		GPU_BACKENDS.find((backend) =>
			installed.some((row) => row.backend === backend),
		) ?? installed[0]?.backend
	);
}

/** The memory a loaded model holds; nothing while it isn't loaded. */
export function memoryOf(
	model: HostedModel,
): { ram: number; vram: number } | undefined {
	return model.state === "loaded"
		? { ram: model.ram_bytes, vram: model.vram_bytes }
		: undefined;
}

export function slotsOf(
	model: HostedModel,
): { busy: number; slots: number } | undefined {
	return model.state === "loaded"
		? { busy: model.slots_busy, slots: model.slots }
		: undefined;
}

/** Equal the way the device compares them: a field left out equals only one left out. */
export function sameSettings(a: ModelSettings, b: ModelSettings): boolean {
	const fields = new Set([...Object.keys(a), ...Object.keys(b)]);
	return [...fields].every(
		(field) =>
			JSON.stringify(a[field as keyof ModelSettings]) ===
			JSON.stringify(b[field as keyof ModelSettings]),
	);
}

export function sameResidency(a: Residency, b: Residency): boolean {
	if (a.mode === "on_demand" && b.mode === "on_demand")
		return a.idle_unload_after_seconds === b.idle_unload_after_seconds;
	return a.mode === b.mode;
}

/** The last 24 whole hours plus the current one, as one hourly stats request. */
export function dayWindow(nowS: number): {
	from: number;
	to: number;
	step: StatsStep;
} {
	const hour = STATS_STEPS.hour;
	const to = (Math.floor(nowS / hour) + 1) * hour;
	return { from: to - 24 * hour, to, step: "hour" };
}

const sum = (values: readonly number[]) =>
	values.reduce((total, value) => total + value, 0);

export interface ModelUsage {
	requests: number;
	/** Generated tokens per second of decode time; absent for embeddings and idle models. */
	tokensPerSecond?: number;
	consumers: ModelStats["consumers"];
}

export function usageOf(stats: ModelStats): ModelUsage {
	const completion = sum(stats.series.completion_tokens);
	const decodeMs = sum(stats.series.decode_ms);
	const consumers = [...stats.consumers].sort(
		(a, b) => b.requests - a.requests,
	);
	return {
		requests: sum(stats.series.requests),
		...(decodeMs > 0 && completion > 0
			? { tokensPerSecond: (completion * 1000) / decodeMs }
			: {}),
		consumers,
	};
}

/** `key` is the device's own name for the caller, unique within one model. */
export type Consumer = { key: string } & (
	| { kind: "you" }
	| { kind: "owner" }
	| { kind: "person"; userId?: string }
	| { kind: "service"; serviceId: string }
);

export interface ConsumerContext {
	/** The viewer owns the device. */
	owner: boolean;
	/** The viewer's own grant on this device, when known. */
	myGrantId?: string;
	/** Grant id → account, from the device's access rules. */
	grantUser(grantId: string): string | undefined;
}

/** Who called a model, busiest first: the viewer, the owner, people with access and services. */
export function consumersOf(
	consumers: ModelStats["consumers"],
	context: ConsumerContext,
): Consumer[] {
	return consumers.map(({ consumer }): Consumer => {
		if (consumer.kind === "owner")
			return context.owner
				? { key: "owner", kind: "you" }
				: { key: "owner", kind: "owner" };
		if (consumer.kind === "placement")
			return {
				key: `placement:${consumer.placement_id}`,
				kind: "service",
				serviceId: consumer.placement_id,
			};
		const key = `grant:${consumer.grant_id}`;
		if (consumer.grant_id === context.myGrantId) return { key, kind: "you" };
		const userId = context.grantUser(consumer.grant_id);
		return userId ? { key, kind: "person", userId } : { key, kind: "person" };
	});
}

const ACTIVE_JOB = new Set<ModelJob["state"]>([
	"queued",
	"fetching",
	"verifying",
	"awaiting_push",
]);

export const jobActive = (job: ModelJob) => ACTIVE_JOB.has(job.state);

export interface JobProgress {
	/** 0–100; absent while it isn't known (queued). */
	percent?: number;
	bytes?: number;
	/** Seconds left at the current rate. */
	etaS?: number;
}

export function jobProgress(job: ModelJob): JobProgress {
	if (job.state === "present") return { percent: 100, bytes: job.size };
	if (job.state === "verifying") return { percent: 100, bytes: job.size };
	if (job.state !== "fetching" && job.state !== "awaiting_push") return {};
	const percent = Math.min(100, (job.bytes / job.size) * 100);
	const rate = job.state === "fetching" ? job.bytes_per_second : undefined;
	return {
		percent,
		bytes: job.bytes,
		...(rate ? { etaS: Math.ceil((job.size - job.bytes) / rate) } : {}),
	};
}

/** Active downloads first, then failed ones, newest first within each. */
export function orderJobs(jobs: readonly ModelJob[]): ModelJob[] {
	const rank = (job: ModelJob) =>
		jobActive(job) ? 0 : job.state === "failed" ? 1 : 2;
	return [...jobs].sort(
		(a, b) => rank(a) - rank(b) || b.updated_at - a.updated_at,
	);
}

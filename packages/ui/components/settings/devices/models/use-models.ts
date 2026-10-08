"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type QueryKey,
	useQueries,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import type { AgentRead } from "../../../../lib/device-management/agent-reads";
import { fleetFacts } from "../../../../lib/device-management/model/device-view";
import { classify } from "../../../../lib/device-management/model/freshness";
import type {
	AgentFeatures,
	Freshness,
	GateResult,
} from "../../../../lib/device-management/model/types";
import {
	type HostedModel,
	MODELS_PAGE_MAX,
	MODEL_OVERVIEW_MAX_RECOMMENDATIONS,
	type ModelJob,
	type ModelStats,
	type ModelStatsInput,
	type ModelsOverview,
	type ModelsSummary,
	type Recommendation,
	probeModelSystem,
	readHostedModels,
	readModelJobs,
	readModelRecommendations,
	readModelStats,
	readModelsOverview,
} from "../../../../lib/device-management/models";
import type { ManagementCall } from "../../../../lib/device-management/telemetry";
import {
	type DeviceFailure,
	classifyDeviceError,
} from "../../../../lib/device-management/workspace/errors";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import type { TabCountTone } from "../primitives/underline-tabs";
import { deviceCall, useAttentionState, useGate } from "../workspace";
import { dayWindow, jobActive, nowCount } from "./models-view";

/*
 * The Models tab's reads (plan §3.5, §3.7). Every read goes out only while the
 * `models_view` gate passes (unlocked, live, permitted, an agent with
 * `model_host`); what was read stays with its age when the session drops and
 * is dropped when the keys lock. Writes invalidate `modelsKeys` through
 * `useModelsAction`.
 */

export const MODELS_POLL_S = 15;
const JOBS_ACTIVE_POLL_S = 5;
const JOBS_IDLE_POLL_S = 30;
const ROW_STATS_POLL_S = 300;
const JOB_PAGES = 4;
const JOB_PAGE = 16;
const MODEL_PAGES = 8;
const RECOMMENDATION_PAGES = 4;

export const modelsKeys = {
	root: (scopeKey: string, deviceId: string) =>
		["devices", scopeKey, "models", deviceId] as const,
	overview: (scopeKey: string, deviceId: string) =>
		[...modelsKeys.root(scopeKey, deviceId), "overview"] as const,
	/** The hosted models after the overview's; `after` is the overview's `next`. */
	models: (scopeKey: string, deviceId: string, after?: string) =>
		[
			...modelsKeys.root(scopeKey, deviceId),
			"models",
			...(after === undefined ? [] : [after]),
		] as const,
	jobs: (scopeKey: string, deviceId: string) =>
		[...modelsKeys.root(scopeKey, deviceId), "jobs"] as const,
	/** Every recommendation, read while the overview's top ones may not be all. */
	recommendations: (scopeKey: string, deviceId: string) =>
		[...modelsKeys.root(scopeKey, deviceId), "recommendations"] as const,
	stats: (scopeKey: string, deviceId: string, input: ModelStatsInput) =>
		[
			...modelsKeys.root(scopeKey, deviceId),
			"stats",
			input.modelId ?? "*",
			input.from,
			input.to,
			input.step,
		] as const,
};

export interface ModelsAccess {
	/** `models_view`: why nothing can be read now, with its fix. */
	gate: GateResult;
	/** Reads may go out now. */
	readable: boolean;
	unlocked: boolean;
	/** A live session is open. */
	live: boolean;
	features: AgentFeatures | undefined;
}

export function useModelsAccess(deviceId: string): ModelsAccess {
	const { input } = useAttentionState();
	const gate = useGate("models_view", deviceId);
	const facts = fleetFacts(input).byId.get(deviceId);
	const unlocked = facts?.keys.state === "unlocked";
	const live = facts?.liveOpen ?? false;
	const features = facts?.inspection?.features;
	return useMemo(
		() => ({ gate, readable: gate.ok, unlocked, live, features }),
		[gate, unlocked, live, features],
	);
}

/** Decrypted reads never outlive the keys they were read with. */
function useDropOnLock(deviceId: string, unlocked: boolean) {
	const { workspace } = useAttentionState();
	const queryClient = useQueryClient();
	useEffect(() => {
		if (unlocked) return;
		queryClient.removeQueries({
			queryKey: modelsKeys.root(workspace.scopeKey, deviceId),
		});
	}, [unlocked, queryClient, workspace.scopeKey, deviceId]);
}

export interface ModelsRead<T> {
	/** The last answer; kept with its age while the session is down, gone when the keys lock. */
	data: T | undefined;
	freshness: Freshness;
	/** The agent doesn't host models: it has no flag for it or answered "unsupported". */
	unsupported: boolean;
	/** The first read is under way. */
	loading: boolean;
	/** The last read failed; `data` is still what was read before. */
	failure: DeviceFailure | undefined;
	refetch(): Promise<void>;
}

type Reader<T> = (
	call: ManagementCall,
	features: AgentFeatures | undefined,
) => Promise<AgentRead<T>>;

interface QuerySpec<T> {
	deviceId: string;
	key: QueryKey;
	enabled: boolean;
	read: Reader<T>;
	refetchInterval: (data: AgentRead<T> | undefined) => number | false;
	staleS: number;
	/** The cadence the stamp promises; `0` for a read made once. */
	cadenceS: number;
}

const queryFnOf =
	<T>(
		workspace: DeviceWorkspace,
		deviceId: string,
		features: AgentFeatures | undefined,
		read: Reader<T>,
	) =>
	() =>
		read(deviceCall(workspace, deviceId, "poll"), features);

/** Hub-corrected unix seconds of a client-clock read time. */
const hubSeconds = (workspace: DeviceWorkspace, atMs: number) =>
	Math.floor(atMs / 1000 - (workspace.clock.hubOffsetS ?? 0));

function useModelsQuery<T>(spec: QuerySpec<T>): ModelsRead<T> {
	const { workspace, input } = useAttentionState();
	const access = useModelsAccess(spec.deviceId);
	useDropOnLock(spec.deviceId, access.unlocked);
	const enabled = spec.enabled && access.readable;
	const query = useQuery<AgentRead<T>>({
		queryKey: spec.key,
		queryFn: queryFnOf(workspace, spec.deviceId, access.features, spec.read),
		enabled,
		refetchInterval: (current) => spec.refetchInterval(current.state.data),
		staleTime: spec.staleS * 1000,
		gcTime: 0,
		retry: false,
		meta: { persist: false },
	});
	const { data, error, dataUpdatedAt, isLoading, refetch } = query;
	const shown = access.unlocked ? data : undefined;
	const failure = useMemo(
		() => (error ? classifyDeviceError(error) : undefined),
		[error],
	);
	const at =
		shown === undefined || !dataUpdatedAt
			? undefined
			: hubSeconds(workspace, dataUpdatedAt);
	const value = shown?.kind === "ok" ? shown.data : undefined;
	const freshness = useMemo<Freshness>(
		() => ({
			...classify("live_inspection", {
				now: Math.max(input.now, at ?? 0),
				at,
				loaded: value !== undefined,
				sessionOpen: access.live,
				locked: !access.unlocked,
				...(failure ? { error: { code: "refresh_failed" as const } } : {}),
			}),
			cadenceS: spec.cadenceS,
		}),
		[
			input.now,
			at,
			value,
			access.live,
			access.unlocked,
			failure,
			spec.cadenceS,
		],
	);
	return useMemo(
		() => ({
			data: value,
			freshness,
			unsupported: shown?.kind === "unsupported",
			loading: enabled && isLoading,
			failure,
			refetch: async () => {
				if (enabled) await refetch();
			},
		}),
		[value, freshness, shown, enabled, isLoading, failure, refetch],
	);
}

const every = (seconds: number | false) => () =>
	seconds === false ? false : seconds * 1000;

export interface PollOptions {
	/** `false`: read once and keep it; the Models tab polls, other places don't. */
	poll?: boolean;
}

/** The overview: hardware, runtimes, hosted models and the top recommendations. */
export function useModelsOverview(
	deviceId: string,
	options: PollOptions = {},
): ModelsRead<ModelsOverview> {
	const { workspace } = useAttentionState();
	return useModelsQuery({
		deviceId,
		key: modelsKeys.overview(workspace.scopeKey, deviceId),
		enabled: true,
		read: readModelsOverview,
		refetchInterval: every(options.poll === false ? false : MODELS_POLL_S),
		staleS: MODELS_POLL_S / 2,
		cadenceS: options.poll === false ? 0 : MODELS_POLL_S,
	});
}

/** Refresh the device's signed runtime catalog, then read its installed and available builds again. */
export function useRuntimeUpdateCheck(deviceId: string, manifestUrl?: string) {
	const { t } = useTranslation("devices");
	const { workspace } = useAttentionState();
	const access = useModelsAccess(deviceId);
	const supported = access.features?.model_runtime_updates === 1;
	const configured = !!manifestUrl;
	const queryClient = useQueryClient();
	const [busy, setBusy] = useState(false);
	const [result, setResult] = useState<
		{ ok: true } | { ok: false; error: string }
	>();
	return {
		gate: access.gate,
		supported,
		configured,
		busy,
		result,
		run: async () => {
			if (!access.readable || !supported || !configured || busy) return;
			setBusy(true);
			setResult(undefined);
			try {
				const response = await probeModelSystem(
					deviceCall(workspace, deviceId, "poll"),
					access.features,
				);
				if (response.kind === "unsupported")
					throw new Error(
						t(
							"devices:models.hardware.checkUnsupported",
							"Update the device agent to check for runtime updates.",
						),
					);
				await queryClient.invalidateQueries({
					queryKey: modelsKeys.root(workspace.scopeKey, deviceId),
				});
				setResult({ ok: true });
			} catch (error) {
				setResult({
					ok: false,
					error: error instanceof Error ? error.message : String(error),
				});
			} finally {
				setBusy(false);
			}
		},
	};
}

/** The hosted models after `after`, page by page, up to `MODEL_PAGES` pages. */
export async function readModelsAfter(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	after: string,
): Promise<AgentRead<HostedModel[]>> {
	const models: HostedModel[] = [];
	let cursor: string | null = after;
	for (let page = 0; cursor !== null && page < MODEL_PAGES; page++) {
		const read = await readHostedModels(call, features, {
			after: cursor,
			limit: MODELS_PAGE_MAX,
		});
		if (read.kind !== "ok") return read;
		models.push(...read.data.models);
		cursor = read.data.next;
	}
	return { kind: "ok", data: models };
}

/**
 * The hosted models the overview couldn't carry: it lists the first ones
 * that fit one reply and names the last as `next`. Nothing to read without.
 */
export function useModelsAfter(
	deviceId: string,
	after: string | null,
	options: PollOptions = {},
): ModelsRead<HostedModel[]> {
	const { workspace } = useAttentionState();
	const cursor = after ?? "";
	return useModelsQuery({
		deviceId,
		key: modelsKeys.models(workspace.scopeKey, deviceId, cursor),
		enabled: after !== null,
		read: (call, features) => readModelsAfter(call, features, cursor),
		refetchInterval: every(options.poll === false ? false : MODELS_POLL_S),
		staleS: MODELS_POLL_S / 2,
		cadenceS: options.poll === false ? 0 : MODELS_POLL_S,
	});
}

/** The overview with every hosted model, its own first, then those read after it. */
export function withAllModels(
	overview: ModelsOverview,
	after: readonly HostedModel[] | undefined,
): ModelsOverview {
	if (!after?.length) return overview;
	const models = new Map(overview.models.map((model) => [model.id, model]));
	for (const model of after)
		if (!models.has(model.id)) models.set(model.id, model);
	return { ...overview, models: [...models.values()] };
}

/** The overview carries only the device's most urgent recommendations; a full set may hold back more. */
export const recommendationsHeldBack = (overview: ModelsOverview | undefined) =>
	(overview?.recommendations.length ?? 0) >= MODEL_OVERVIEW_MAX_RECOMMENDATIONS;

/** Every recommendation from the first, page by page, up to `RECOMMENDATION_PAGES` pages. */
export async function readAllModelRecommendations(
	call: ManagementCall,
	features: AgentFeatures | undefined,
): Promise<AgentRead<Recommendation[]>> {
	const recommendations: Recommendation[] = [];
	let after: string | undefined;
	for (let page = 0; page < RECOMMENDATION_PAGES; page++) {
		const read = await readModelRecommendations(call, features, {
			...(after ? { after } : {}),
			limit: MODELS_PAGE_MAX,
		});
		if (read.kind !== "ok") return read;
		recommendations.push(...read.data.recommendations);
		if (read.data.next === null) break;
		after = read.data.next;
	}
	return { kind: "ok", data: recommendations };
}

/**
 * Every recommendation while the overview's top ones may not be all of them;
 * nothing is read otherwise. Polled with the overview, so fixes stay current.
 */
export function useModelsRecommendations(
	deviceId: string,
	overview: ModelsOverview | undefined,
	options: PollOptions = {},
): ModelsRead<Recommendation[]> {
	const { workspace } = useAttentionState();
	return useModelsQuery({
		deviceId,
		key: modelsKeys.recommendations(workspace.scopeKey, deviceId),
		enabled: recommendationsHeldBack(overview),
		read: readAllModelRecommendations,
		refetchInterval: every(options.poll === false ? false : MODELS_POLL_S),
		staleS: MODELS_POLL_S / 2,
		cadenceS: options.poll === false ? 0 : MODELS_POLL_S,
	});
}

/** The overview with the full list of recommendations once it was read; its own top ones before. */
export function withAllRecommendations(
	overview: ModelsOverview,
	all: readonly Recommendation[] | undefined,
): ModelsOverview {
	if (!all?.length || !recommendationsHeldBack(overview)) return overview;
	return { ...overview, recommendations: [...all] };
}

export interface ModelJobs {
	jobs: ModelJob[];
	/** False when the device holds more jobs than the pages read. */
	complete: boolean;
}

/** Every job, page by page, up to `JOB_PAGES` pages. */
export async function readAllModelJobs(
	call: ManagementCall,
	features: AgentFeatures | undefined,
): Promise<AgentRead<ModelJobs>> {
	const jobs: ModelJob[] = [];
	let after: string | undefined;
	for (let page = 0; page < JOB_PAGES; page++) {
		const read = await readModelJobs(call, features, {
			...(after ? { after } : {}),
			limit: JOB_PAGE,
		});
		if (read.kind !== "ok") return read;
		jobs.push(...read.data.jobs);
		if (read.data.next === null)
			return { kind: "ok", data: { jobs, complete: true } };
		after = read.data.next;
	}
	return { kind: "ok", data: { jobs, complete: false } };
}

const jobsInterval =
	(poll: boolean) =>
	(data: AgentRead<ModelJobs> | undefined): number | false => {
		if (!poll) return false;
		const active = data?.kind === "ok" && data.data.jobs.some(jobActive);
		return (active ? JOBS_ACTIVE_POLL_S : JOBS_IDLE_POLL_S) * 1000;
	};

/** Downloads of model files and runtime packs: polled faster while one runs. */
export function useModelsJobs(
	deviceId: string,
	options: PollOptions = {},
): ModelsRead<ModelJobs> {
	const { workspace } = useAttentionState();
	const poll = options.poll !== false;
	return useModelsQuery({
		deviceId,
		key: modelsKeys.jobs(workspace.scopeKey, deviceId),
		enabled: true,
		read: readAllModelJobs,
		refetchInterval: jobsInterval(poll),
		staleS: JOBS_ACTIVE_POLL_S / 2,
		cadenceS: poll ? JOBS_ACTIVE_POLL_S : 0,
	});
}

const statsReader =
	(input: ModelStatsInput): Reader<ModelStats> =>
	async (call, features) =>
		readModelStats(call, features, input);

/**
 * Usage statistics of one model, or of all when `modelId` is absent; `null`
 * reads nothing. Read once per query unless `pollS` is set. A bulk read:
 * rendering is the statistics view's.
 */
export function useModelsStats(
	deviceId: string,
	query: ModelStatsInput | null,
	options: { pollS?: number } = {},
): ModelsRead<ModelStats> {
	const { workspace } = useAttentionState();
	const idle: ModelStatsInput = { from: 0, to: 0, step: "hour" };
	const input = query ?? idle;
	return useModelsQuery({
		deviceId,
		key: modelsKeys.stats(workspace.scopeKey, deviceId, input),
		enabled: query !== null,
		read: statsReader(input),
		refetchInterval: every(options.pollS ?? false),
		staleS: options.pollS ?? 60,
		cadenceS: options.pollS ?? 0,
	});
}

/** The last 24 hours of each listed model, for the table's request and speed columns. */
export function useModelsDayUsage(
	deviceId: string,
	modelIds: readonly string[],
): ReadonlyMap<string, ModelStats> {
	const { workspace, input } = useAttentionState();
	const access = useModelsAccess(deviceId);
	const window = dayWindow(input.now);
	const results = useQueries({
		queries: modelIds.map((modelId) => {
			const stats: ModelStatsInput = { modelId, ...window };
			return {
				queryKey: modelsKeys.stats(workspace.scopeKey, deviceId, stats),
				queryFn: queryFnOf(
					workspace,
					deviceId,
					access.features,
					statsReader(stats),
				),
				enabled: access.readable,
				refetchInterval: ROW_STATS_POLL_S * 1000,
				staleTime: (ROW_STATS_POLL_S * 1000) / 2,
				gcTime: 0,
				retry: false,
				meta: { persist: false },
			};
		}),
	});
	const answers = results.map((result) =>
		access.unlocked && result.data?.kind === "ok" ? result.data.data : null,
	);
	const ids = modelIds.join("|");
	const signature = answers
		.map((stats, index) => (stats ? results[index]?.dataUpdatedAt : 0))
		.join("|");
	// biome-ignore lint/correctness/useExhaustiveDependencies: `ids` and `signature` stand for the models and their answers
	return useMemo(() => {
		const byModel = new Map<string, ModelStats>();
		for (const [index, modelId] of modelIds.entries()) {
			const stats = answers[index];
			if (stats) byModel.set(modelId, stats);
		}
		return byModel;
	}, [ids, signature]);
}

export interface ModelsSnapshot {
	summary: ModelsSummary;
	/** Unix seconds the device reported it. */
	observedAt: number;
	freshness: Freshness;
}

/**
 * The model host's counters from the encrypted metrics snapshot (plan §3.6):
 * while unlocked the newest one, while locked what the keys kept when they
 * locked. Only model readers' snapshots carry them.
 */
export function useModelsSnapshot(
	deviceId: string,
): ModelsSnapshot | undefined {
	const { input } = useAttentionState();
	const fleet = input.fleet[deviceId];
	const keys = fleetFacts(input).byId.get(deviceId)?.keys;
	const unlocked = keys?.state === "unlocked";
	const snapshot = unlocked
		? fleet?.metrics?.find(
				(entry) => entry.scope.kind === "device" && entry.models,
			)
		: undefined;
	const kept = unlocked ? undefined : keys?.lockedSummary?.models;
	const summary = snapshot?.models ?? kept?.summary;
	const observedAt = snapshot?.observedAt ?? kept?.readAt;
	const error = unlocked ? fleet?.freshness.metrics.error : undefined;
	return useMemo(() => {
		if (!summary || observedAt === undefined) return undefined;
		const freshness = classify("fleet_metrics", {
			now: Math.max(input.now, observedAt),
			at: observedAt,
			loaded: true,
			locked: !unlocked,
			...(error ? { error } : {}),
		});
		return { summary, observedAt, freshness };
	}, [summary, observedAt, unlocked, error, input.now]);
}

/** A read the Models tab made, from the cache only: this reads nothing by itself. */
function useCachedModelsRead<T>(
	deviceId: string,
	key: QueryKey,
	read: Reader<T>,
): AgentRead<T> | undefined {
	const { workspace } = useAttentionState();
	const access = useModelsAccess(deviceId);
	const { data } = useQuery({
		queryKey: key,
		queryFn: queryFnOf(workspace, deviceId, access.features, read),
		enabled: false,
		gcTime: 0,
		meta: { persist: false },
	});
	return access.unlocked ? data : undefined;
}

/**
 * The Models tab's badge: recommendations filed under "now" plus models that
 * failed. Recommendations come from the tab's last overview (and the full list
 * when it read one); failed models from the overview, else from the encrypted
 * metrics snapshot, so the badge shows before the tab is opened. Reads nothing
 * by itself.
 */
export function useModelsAttention(
	deviceId: string,
): { count: number; tone: TabCountTone } | undefined {
	const { workspace } = useAttentionState();
	const access = useModelsAccess(deviceId);
	useDropOnLock(deviceId, access.unlocked);
	const overview = useCachedModelsRead(
		deviceId,
		modelsKeys.overview(workspace.scopeKey, deviceId),
		readModelsOverview,
	);
	const all = useCachedModelsRead(
		deviceId,
		modelsKeys.recommendations(workspace.scopeKey, deviceId),
		readAllModelRecommendations,
	);
	const snapshot = useModelsSnapshot(deviceId);
	if (!access.unlocked) return undefined;
	const read = overview?.kind === "ok" ? overview.data : undefined;
	const recommendations = read
		? withAllRecommendations(read, all?.kind === "ok" ? all.data : undefined)
				.recommendations
		: [];
	const failed = read?.summary.failed ?? snapshot?.summary.failed ?? 0;
	const count = nowCount(recommendations) + failed;
	return count ? { count, tone: "critical" } : undefined;
}

import { create } from "zustand";
import type { ILogLevel, ILogMetadata, IRunLogSummary } from "../lib";
import type { IBackendState } from "./backend-state";

export interface ILogAggregationFilter {
	appId: string;
	boardId: string;
	nodeId?: string;
	from?: number;
	to?: number;
	status?: ILogLevel;
	limit?: number;
	offset?: number;
	lastMeta?: ILogMetadata;
}

/** Aggregated per-node activity across the currently listed runs. */
export interface INodeHeat {
	/** How many runs visited the node. */
	visits: number;
	/** Runs where the node logged at Error level or above. */
	errors: number;
}

export interface IBoardHeatmap {
	nodes: Record<string, INodeHeat>;
	maxVisits: number;
	runCount: number;
}

/**
 * Directly-follows aggregation over the runs list: each run's visited-node
 * summary (`[nodeId, maxLogLevel][]`) is unioned into visit and error counts
 * per node — the data behind the board activity heatmap.
 */
export function aggregateHeatmap(runs: ILogMetadata[]): IBoardHeatmap {
	const nodes: Record<string, INodeHeat> = {};
	let maxVisits = 0;
	for (const run of runs) {
		for (const entry of run.nodes ?? []) {
			const [nodeId, level] = entry;
			if (typeof nodeId !== "string") continue;
			const heat = nodes[nodeId] ?? { visits: 0, errors: 0 };
			heat.visits += 1;
			if (typeof level === "number" && level >= 3) heat.errors += 1;
			nodes[nodeId] = heat;
			if (heat.visits > maxVisits) maxVisits = heat.visits;
		}
	}
	return { nodes, maxVisits, runCount: runs.length };
}

interface ILogAggregationState {
	currentLogs: ILogMetadata[];
	filter?: ILogAggregationFilter;
	currentMetadata?: ILogMetadata;
	isLoading: boolean;
	/** When enabled, the board renders aggregated run activity per node. */
	heatmapEnabled: boolean;
	heatmap?: IBoardHeatmap;
	/** Log summary of `currentMetadata`, loaded by the Logs panel. */
	currentSummary?: IRunLogSummary;
	currentSummaryRunId?: string;
	refetchLogs: (backend: IBackendState) => Promise<void>;
	setFilter(
		backend: IBackendState,
		filter: ILogAggregationFilter,
	): Promise<void>;
	setCurrentMetadata: (meta?: ILogMetadata) => void;
	setCurrentSummary: (runId: string, summary?: IRunLogSummary | null) => void;
	setHeatmapEnabled: (enabled: boolean) => void;
}

function withHeatmap(
	runs: ILogMetadata[],
	enabled: boolean,
): { currentLogs: ILogMetadata[]; heatmap?: IBoardHeatmap } {
	return {
		currentLogs: runs,
		heatmap: enabled ? aggregateHeatmap(runs) : undefined,
	};
}

export interface INodeLogCounts {
	total: number;
	problems: number;
	warnings: number;
}

/** Per-node counts from the current run's summary. Select it through `useShallow`. */
export function nodeLogCounts(
	state: Pick<
		ILogAggregationState,
		"currentSummary" | "currentSummaryRunId" | "currentMetadata"
	>,
	nodeId: string,
): INodeLogCounts | undefined {
	if (state.currentMetadata?.run_id !== state.currentSummaryRunId) {
		return undefined;
	}
	const levels = state.currentSummary?.nodes?.[nodeId];
	if (!levels) return undefined;
	let total = 0;
	for (const count of levels) total += count ?? 0;
	return {
		total,
		problems: (levels[3] ?? 0) + (levels[4] ?? 0),
		warnings: levels[2] ?? 0,
	};
}

export const useLogAggregation = create<ILogAggregationState>((set, get) => ({
	currentLogs: [],
	filter: undefined,
	currentMetadata: undefined,
	currentSummary: undefined,
	currentSummaryRunId: undefined,
	isLoading: false,
	heatmapEnabled: false,
	heatmap: undefined,
	setFilter: async (backend: IBackendState, filter: ILogAggregationFilter) => {
		const currentFilter = get().filter;
		const boardChanged =
			currentFilter?.appId !== filter.appId ||
			currentFilter?.boardId !== filter.boardId;

		// Clear currentMetadata when board changes to avoid showing stale logs
		if (boardChanged) {
			set({ filter, currentMetadata: undefined, isLoading: true });
		} else {
			set({ filter, isLoading: true });
		}

		try {
			const runs = await backend.boardState.listRuns(
				filter.appId,
				filter.boardId,
				filter.nodeId,
				filter.from,
				filter.to,
				filter.status,
				filter.lastMeta,
				filter.offset,
				filter.limit,
				// Per-node summaries cost an extra query — only when the heatmap needs them.
				get().heatmapEnabled,
			);

			set({
				...withHeatmap(
					runs.toSorted((a, b) => b.start - a.start),
					get().heatmapEnabled,
				),
				isLoading: false,
			});
		} catch {
			set({ isLoading: false });
		}
	},
	setCurrentMetadata: (meta?: ILogMetadata) => {
		if (meta?.run_id === get().currentSummaryRunId) {
			set({ currentMetadata: meta });
			return;
		}
		set({
			currentMetadata: meta,
			currentSummary: undefined,
			currentSummaryRunId: undefined,
		});
	},
	setCurrentSummary: (runId: string, summary?: IRunLogSummary | null) => {
		if (get().currentMetadata?.run_id !== runId) return;
		set({ currentSummary: summary ?? undefined, currentSummaryRunId: runId });
	},
	setHeatmapEnabled: (enabled: boolean) => {
		set({
			heatmapEnabled: enabled,
			heatmap: enabled ? aggregateHeatmap(get().currentLogs) : undefined,
		});
	},
	refetchLogs: async (backend: IBackendState) => {
		const { filter } = get();

		if (!filter) {
			return;
		}

		set({ isLoading: true });

		try {
			const runs = await backend.boardState.listRuns(
				filter.appId,
				filter.boardId,
				filter.nodeId,
				filter.from,
				filter.to,
				filter.status,
				filter.lastMeta,
				filter.offset,
				filter.limit,
				// Per-node summaries cost an extra query — only when the heatmap needs them.
				get().heatmapEnabled,
			);

			set({
				...withHeatmap(
					runs.toSorted((a, b) => b.start - a.start),
					get().heatmapEnabled,
				),
				isLoading: false,
			});
		} catch {
			set({ isLoading: false });
		}
	},
}));

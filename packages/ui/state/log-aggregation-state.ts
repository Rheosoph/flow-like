import { create } from "zustand";
import type { ILogLevel, ILogMetadata } from "../lib";
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
	refetchLogs: (backend: IBackendState) => Promise<void>;
	setFilter(
		backend: IBackendState,
		filter: ILogAggregationFilter,
	): Promise<void>;
	setCurrentMetadata: (meta?: ILogMetadata) => void;
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

export const useLogAggregation = create<ILogAggregationState>((set, get) => ({
	currentLogs: [],
	filter: undefined,
	currentMetadata: undefined,
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
		set({ currentMetadata: meta });
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

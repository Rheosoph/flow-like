import { create } from "zustand";

export interface IRunExecution {
	eventIds: string[];
	boardId: string;
	/** Currently executing nodes (unique) */
	nodes: Set<string>;
	/** Nodes that have finished executing (unique) */
	already_executed: Set<string>;
	/** Timestamp (ms since epoch) of the last node update event */
	lastNodeUpdateMs: number;
	/** Total number of node executions started (counts duplicates for loops) */
	totalExecutionsStarted: number;
	/** Total number of node executions completed (counts duplicates for loops) */
	totalExecutionsCompleted: number;
	/** The most recently active node ID */
	lastActiveNodeId: string | undefined;
}

interface IRunExecutionState {
	runs: Map<string, IRunExecution>;
	pushUpdate(runId: string, events: IRunUpdateEvent[]): void;
	addRun: (runId: string, boardId: string, eventIds: string[]) => Promise<void>;
	removeRun: (runId: string) => void;
	addNodesOnRun: (runId: string, nodeIds: string[]) => void;
	removeNodesOnRun: (runId: string, nodeIds: string[]) => void;
	getTimeSinceLastUpdate: (runId: string) => number | null;
}

export interface IRunUpdateEvent {
	runId: string;
	nodeIds: string[];
	method: "remove" | "add" | "update";
}

function withStartedNodes(
	run: IRunExecution,
	nodeIds: readonly string[],
	now: number,
): IRunExecution {
	const nodes = new Set(run.nodes);
	for (const nodeId of nodeIds) nodes.add(nodeId);
	return {
		...run,
		nodes,
		lastNodeUpdateMs: now,
		// Count every execution start, not just unique nodes
		totalExecutionsStarted: run.totalExecutionsStarted + nodeIds.length,
		lastActiveNodeId: nodeIds.at(-1) ?? run.lastActiveNodeId,
	};
}

function withFinishedNodes(
	run: IRunExecution,
	nodeIds: readonly string[],
	now: number,
): IRunExecution {
	const nodes = new Set(run.nodes);
	const alreadyExecuted = new Set(run.already_executed);
	for (const nodeId of nodeIds) {
		nodes.delete(nodeId);
		alreadyExecuted.add(nodeId);
	}
	return {
		...run,
		nodes,
		already_executed: alreadyExecuted,
		lastNodeUpdateMs: now,
		// Count every execution completion, not just unique nodes
		totalExecutionsCompleted: run.totalExecutionsCompleted + nodeIds.length,
	};
}

/** Node ids per run of the batch's start events (`started`) or of its other events. */
function groupNodeIds(
	events: readonly IRunUpdateEvent[],
	started: boolean,
): Map<string, string[]> {
	const grouped = new Map<string, string[]>();
	for (const event of events) {
		if ((event.method === "add") !== started) continue;
		const nodeIds = grouped.get(event.runId);
		if (!nodeIds) {
			grouped.set(event.runId, [...event.nodeIds]);
			continue;
		}
		for (const nodeId of event.nodeIds) nodeIds.push(nodeId);
	}
	return grouped;
}

export const useRunExecutionStore = create<IRunExecutionState>((set, get) => ({
	runs: new Map(),
	/** One store write per batch: subscribers re-render once, not once per event kind. */
	pushUpdate: (_runId: string, events: IRunUpdateEvent[]) => {
		const started = groupNodeIds(events, true);
		const finished = groupNodeIds(events, false);
		if (started.size === 0 && finished.size === 0) return;
		const now = Date.now();
		set((state) => {
			let runs: Map<string, IRunExecution> | undefined;
			for (const runId of new Set([...started.keys(), ...finished.keys()])) {
				let run = state.runs.get(runId);
				if (!run) continue;
				run = { ...run, lastNodeUpdateMs: now };
				const startedIds = started.get(runId);
				if (startedIds) run = withStartedNodes(run, startedIds, now);
				const finishedIds = finished.get(runId);
				if (finishedIds) run = withFinishedNodes(run, finishedIds, now);
				runs ??= new Map(state.runs);
				runs.set(runId, run);
			}
			return runs ? { runs } : state;
		});
	},
	addRun: async (runId: string, boardId: string, eventIds: string[]) => {
		if (get().runs.has(runId)) {
			return;
		}

		set((state) => {
			const runs = new Map(state.runs);
			runs.set(runId, {
				eventIds,
				boardId,
				nodes: new Set(),
				already_executed: new Set(),
				lastNodeUpdateMs: Date.now(),
				totalExecutionsStarted: 0,
				totalExecutionsCompleted: 0,
				lastActiveNodeId: undefined,
			});
			return { runs };
		});
	},

	removeRun: (runId: string) =>
		set((state) => {
			const runs = new Map(state.runs);
			runs.delete(runId);
			return { runs };
		}),

	addNodesOnRun: (runId: string, nodeIds: string[]) =>
		set((state) => {
			const run = state.runs.get(runId);
			if (!run) return state;
			const runs = new Map(state.runs);
			runs.set(runId, withStartedNodes(run, nodeIds, Date.now()));
			return { runs };
		}),

	removeNodesOnRun: (runId: string, nodeIds: string[]) =>
		set((state) => {
			const run = state.runs.get(runId);
			if (!run) return state;
			const runs = new Map(state.runs);
			runs.set(runId, withFinishedNodes(run, nodeIds, Date.now()));
			return { runs };
		}),

	getTimeSinceLastUpdate: (runId: string) => {
		const run = get().runs.get(runId);
		if (!run) return null;
		return Date.now() - run.lastNodeUpdateMs;
	},
}));

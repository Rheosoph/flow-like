/**
 * Live run traces and post-run stats for the FlowScript editor, mapped from
 * node ids to lines through the anchor index.
 *
 * Everything here is pure derivation (plus one timer coalescer) so the panel
 * can subscribe to the run-execution store imperatively — hundreds of node
 * events per second must not re-render the panel — and only touch Monaco
 * decorations when the mapped line sets actually change (compare `key`).
 */

import type { FlowScriptAnchor } from "./flowscript-anchors";

export interface FlowScriptRunLike {
	boardId: string;
	nodes: ReadonlySet<string>;
	already_executed: ReadonlySet<string>;
}

export interface FlowScriptRemoteExecutionLike {
	sub: string;
	executingNodes: readonly string[];
}

export interface FlowScriptRemoteTraceLine {
	line: number;
	/** Peer palette slot; undefined renders the neutral remote style. */
	slot?: number;
}

export interface FlowScriptRunTraceLines {
	executing: number[];
	done: number[];
	remote: FlowScriptRemoteTraceLine[];
	/** Canonical identity of the three line sets — equal key ⇒ skip the decoration write. */
	key: string;
}

export interface DeriveRunTraceInput {
	boardId: string;
	runs: ReadonlyMap<string, FlowScriptRunLike>;
	remoteExecutions?: readonly FlowScriptRemoteExecutionLike[];
	firstLineById: ReadonlyMap<string, number>;
	slotFor?: (sub: string) => number | undefined;
}

const numericAsc = (a: number, b: number) => a - b;

/**
 * Local runs win styling conflicts: a line stays "executing" over "done", and
 * a remote wash never paints over a locally executing line.
 */
export function deriveRunTraceLines({
	boardId,
	runs,
	remoteExecutions,
	firstLineById,
	slotFor,
}: DeriveRunTraceInput): FlowScriptRunTraceLines {
	const executingLines = new Set<number>();
	const doneLines = new Set<number>();
	for (const run of runs.values()) {
		if (run.boardId !== boardId) continue;
		for (const nodeId of run.nodes) {
			const line = firstLineById.get(nodeId);
			if (line) executingLines.add(line);
		}
		for (const nodeId of run.already_executed) {
			const line = firstLineById.get(nodeId);
			if (line) doneLines.add(line);
		}
	}
	for (const line of executingLines) doneLines.delete(line);

	const remote: FlowScriptRemoteTraceLine[] = [];
	const remoteSeen = new Set<number>();
	for (const execution of remoteExecutions ?? []) {
		const slot = slotFor?.(execution.sub);
		for (const nodeId of execution.executingNodes) {
			const line = firstLineById.get(nodeId);
			if (!line || executingLines.has(line) || remoteSeen.has(line)) continue;
			remoteSeen.add(line);
			remote.push({ line, slot });
		}
	}

	const executing = [...executingLines].sort(numericAsc);
	const done = [...doneLines].sort(numericAsc);
	remote.sort((a, b) => a.line - b.line);
	const key = `e:${executing.join(",")}|d:${done.join(",")}|r:${remote
		.map((entry) => `${entry.line}@${entry.slot ?? "n"}`)
		.join(",")}`;
	return { executing, done, remote, key };
}

export interface FlowScriptRunStatsInlay {
	line: number;
	/** Symbolic (`· 12× ⚠3`) on purpose — no words, so nothing to translate. */
	text: string;
}

export interface FlowScriptNodeHeatLike {
	visits: number;
	errors: number;
}

/**
 * Post-run inlay per anchored statement line, from the board heatmap
 * (`useLogAggregation` aggregates run visit/error counts per node — the
 * heatmap carries no durations). One inlay per line; when several anchors on
 * distinct lines share a node id each line shows that node's counts.
 */
export function deriveRunStatsInlays(
	anchors: readonly FlowScriptAnchor[],
	nodeHeat: Readonly<Record<string, FlowScriptNodeHeatLike>>,
): FlowScriptRunStatsInlay[] {
	const inlays: FlowScriptRunStatsInlay[] = [];
	for (const anchor of anchors) {
		if (anchor.kind !== "node") continue;
		const heat = nodeHeat[anchor.id];
		if (!heat || heat.visits <= 0) continue;
		const errors = heat.errors > 0 ? ` ⚠${heat.errors}` : "";
		inlays.push({ line: anchor.line, text: `  · ${heat.visits}×${errors}` });
	}
	inlays.sort((a, b) => a.line - b.line);
	return inlays;
}

export function runStatsKey(
	inlays: readonly FlowScriptRunStatsInlay[],
): string {
	return inlays.map((inlay) => `${inlay.line}:${inlay.text}`).join("|");
}

export interface CoalescedInvoker {
	trigger: () => void;
	dispose: () => void;
}

type Schedule = (callback: () => void, delayMs: number) => unknown;
type Cancel = (handle: unknown) => void;

/**
 * Trailing-edge coalescer for store subscriptions: a burst of triggers inside
 * one window costs a single invocation, `delayMs` after the first trigger.
 * Scheduler injectable for tests.
 */
export function createCoalescedInvoker(
	invoke: () => void,
	delayMs: number,
	schedule: Schedule = (callback, ms) => setTimeout(callback, ms),
	cancel: Cancel = (handle) =>
		clearTimeout(handle as ReturnType<typeof setTimeout>),
): CoalescedInvoker {
	let pending: unknown;
	let disposed = false;
	return {
		trigger: () => {
			if (disposed || pending !== undefined) return;
			pending = schedule(() => {
				pending = undefined;
				if (!disposed) invoke();
			}, delayMs);
		},
		dispose: () => {
			disposed = true;
			if (pending !== undefined) {
				cancel(pending);
				pending = undefined;
			}
		},
	};
}

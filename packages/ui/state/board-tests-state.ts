import { create } from "zustand";
import type { IBoardTestResult } from "../lib/board-tests";

export type IBoardTestStatus = "running" | "pass" | "fail" | "error";

export interface IBoardTestEntry {
	status: IBoardTestStatus;
	result?: IBoardTestResult;
	startedAt: number;
}

interface IBoardTestsState {
	/** boardId → test node id → latest run entry. Kept so results survive panel tab switches. */
	entries: Record<string, Record<string, IBoardTestEntry>>;
	begin: (boardId: string, nodeIds: string[]) => void;
	complete: (boardId: string, result: IBoardTestResult) => void;
	clear: (boardId: string) => void;
}

export const useBoardTestsStore = create<IBoardTestsState>((set) => ({
	entries: {},
	begin: (boardId, nodeIds) =>
		set((state) => {
			const board = { ...(state.entries[boardId] ?? {}) };
			const startedAt = Date.now();
			for (const nodeId of nodeIds) {
				board[nodeId] = { status: "running", startedAt };
			}
			return { entries: { ...state.entries, [boardId]: board } };
		}),
	complete: (boardId, result) =>
		set((state) => {
			const board = { ...(state.entries[boardId] ?? {}) };
			board[result.nodeId] = {
				status: result.verdict,
				result,
				startedAt: board[result.nodeId]?.startedAt ?? Date.now(),
			};
			return { entries: { ...state.entries, [boardId]: board } };
		}),
	clear: (boardId) =>
		set((state) => {
			if (!state.entries[boardId]) return state;
			const entries = { ...state.entries };
			delete entries[boardId];
			return { entries };
		}),
}));

/** Summarize entries, ignoring stale ones whose node was deleted or renamed away. */
export function boardTestSummary(
	entries: Record<string, IBoardTestEntry> | undefined,
	liveNodeIds?: ReadonlySet<string>,
): { running: number; passed: number; failed: number } {
	let running = 0;
	let passed = 0;
	let failed = 0;
	for (const [nodeId, entry] of Object.entries(entries ?? {})) {
		if (liveNodeIds && !liveNodeIds.has(nodeId)) continue;
		if (entry.status === "running") running += 1;
		else if (entry.status === "pass") passed += 1;
		else failed += 1;
	}
	return { running, passed, failed };
}

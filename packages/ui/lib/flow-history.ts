import type { IGenericCommand } from "./schema";
import type { IBoard } from "./schema/flow/board";

/**
 * One recorded edit: the executed command batch, serialized once at record time.
 *
 * The batch is kept as a compact JSON string rather than an object graph — it is only ever
 * parsed again when it is replayed, the same string is what persistence writes, and its length
 * is the memory budget.
 */
export interface HistoryEntry {
	/** Monotonic per board; doubles as the persisted row id and never repeats within a board. */
	seq: number;
	/** Epoch milliseconds. */
	at: number;
	payload: string;
	bytes: number;
}

/**
 * A linear history with a cursor. `entries[0, cursor)` are applied to the board and can be undone
 * newest-first; `entries[cursor, length)` were undone and can be redone oldest-first. Recording a
 * new edit discards the redo tail — a branch that no longer matches the board.
 */
export interface HistoryTimeline {
	entries: readonly HistoryEntry[];
	cursor: number;
	nextSeq: number;
}

export interface HistorySummary {
	canUndo: boolean;
	canRedo: boolean;
	undoDepth: number;
	redoDepth: number;
}

export interface HistoryStep {
	timeline: HistoryTimeline;
	entry: HistoryEntry;
}

export interface HistoryPush extends HistoryStep {
	/** Entries no longer part of the timeline: the discarded redo tail plus anything evicted. */
	removed: HistoryEntry[];
}

export const HISTORY_MAX_ENTRIES = 200;
export const HISTORY_MAX_BYTES = 8 * 1024 * 1024;

export const emptyTimeline = (nextSeq = 1): HistoryTimeline => ({
	entries: [],
	cursor: 0,
	nextSeq,
});

export const encodeCommands = (commands: readonly IGenericCommand[]): string =>
	JSON.stringify(commands);

export const decodeEntry = (entry: HistoryEntry): IGenericCommand[] => {
	const parsed: unknown = JSON.parse(entry.payload);
	if (!Array.isArray(parsed)) {
		throw new Error(
			`History entry ${entry.seq} does not hold a command batch (got ${typeof parsed})`,
		);
	}
	return parsed as IGenericCommand[];
};

export const summarizeTimeline = (
	timeline: HistoryTimeline,
): HistorySummary => ({
	canUndo: timeline.cursor > 0,
	canRedo: timeline.cursor < timeline.entries.length,
	undoDepth: timeline.cursor,
	redoDepth: timeline.entries.length - timeline.cursor,
});

/**
 * Rebuild a timeline from persisted rows. Rows are sorted by `seq`; `appliedSeq` names the newest
 * entry that is applied to the board (0 = everything is undone). A cursor that points past a row
 * that no longer exists snaps to the nearest surviving one.
 */
export const timelineFromRows = (
	rows: readonly HistoryEntry[],
	appliedSeq: number,
	nextSeq?: number,
): HistoryTimeline => {
	const entries = [...rows].sort((a, b) => a.seq - b.seq);
	let cursor = 0;
	while (cursor < entries.length && entries[cursor].seq <= appliedSeq) cursor++;
	const highest = entries.length > 0 ? entries[entries.length - 1].seq : 0;
	return {
		entries,
		cursor,
		nextSeq: Math.max(nextSeq ?? 0, highest + 1, 1),
	};
};

export const appliedSeq = (timeline: HistoryTimeline): number =>
	timeline.cursor > 0 ? timeline.entries[timeline.cursor - 1].seq : 0;

interface Budget {
	maxEntries: number;
	maxBytes: number;
}

const DEFAULT_BUDGET: Budget = {
	maxEntries: HISTORY_MAX_ENTRIES,
	maxBytes: HISTORY_MAX_BYTES,
};

/**
 * Drop the oldest applied entries until the timeline fits. The newest entry always survives, even
 * when it alone exceeds the byte budget — an edit that was recorded must stay undoable.
 */
const evict = (
	entries: HistoryEntry[],
	budget: Budget,
): { kept: HistoryEntry[]; evicted: HistoryEntry[] } => {
	let bytes = 0;
	for (const entry of entries) bytes += entry.bytes;
	let drop = 0;
	while (
		entries.length - drop > 1 &&
		(entries.length - drop > budget.maxEntries || bytes > budget.maxBytes)
	) {
		bytes -= entries[drop].bytes;
		drop++;
	}
	return drop === 0
		? { kept: entries, evicted: [] }
		: { kept: entries.slice(drop), evicted: entries.slice(0, drop) };
};

export const pushEntry = (
	timeline: HistoryTimeline,
	commands: readonly IGenericCommand[],
	at: number,
	budget: Budget = DEFAULT_BUDGET,
): HistoryPush => {
	const payload = encodeCommands(commands);
	const entry: HistoryEntry = {
		seq: timeline.nextSeq,
		at,
		payload,
		bytes: payload.length,
	};
	const truncated = timeline.entries.slice(timeline.cursor);
	const { kept, evicted } = evict(
		[...timeline.entries.slice(0, timeline.cursor), entry],
		budget,
	);
	return {
		timeline: {
			entries: kept,
			cursor: kept.length,
			nextSeq: entry.seq + 1,
		},
		entry,
		removed: [...truncated, ...evicted],
	};
};

export const stepBack = (
	timeline: HistoryTimeline,
): HistoryStep | undefined => {
	if (timeline.cursor === 0) return undefined;
	const cursor = timeline.cursor - 1;
	return { timeline: { ...timeline, cursor }, entry: timeline.entries[cursor] };
};

export const stepForward = (
	timeline: HistoryTimeline,
): HistoryStep | undefined => {
	if (timeline.cursor >= timeline.entries.length) return undefined;
	return {
		timeline: { ...timeline, cursor: timeline.cursor + 1 },
		entry: timeline.entries[timeline.cursor],
	};
};

export const withCursor = (
	timeline: HistoryTimeline,
	cursor: number,
): HistoryTimeline => ({
	...timeline,
	cursor: Math.max(0, Math.min(cursor, timeline.entries.length)),
});

/**
 * The board refused to replay `entry` repeatedly: it no longer describes anything the board
 * holds. Drop it so the next attempt moves on to the neighbour instead of failing forever.
 */
export const discardEntry = (
	timeline: HistoryTimeline,
	entry: HistoryEntry,
): HistoryTimeline => {
	const index = timeline.entries.findIndex((item) => item.seq === entry.seq);
	if (index === -1) return timeline;
	return {
		...timeline,
		entries: [
			...timeline.entries.slice(0, index),
			...timeline.entries.slice(index + 1),
		],
		cursor: index < timeline.cursor ? timeline.cursor - 1 : timeline.cursor,
	};
};

const TRANSIENT_STATUS = new Set([408, 423, 425, 429]);

const TRANSIENT_MESSAGE =
	/network|failed to fetch|timed? ?out|offline|board_locked|mutation lease|paused while flowpilot|reserved by flowpilot|not initialized/i;

/**
 * Whether a replay failure says nothing about the entry itself — the request never reached a
 * decision (transport, lock, auth, an unavailable backend) — as opposed to the board refusing
 * the batch. Only refusals may count against an entry.
 */
export const isTransientReplayError = (error: unknown): boolean => {
	if (error instanceof TypeError) return true;
	const candidate = error as { status?: unknown; message?: unknown } | null;
	const status =
		typeof candidate?.status === "number" ? candidate.status : undefined;
	if (status !== undefined) {
		if (status >= 500 || TRANSIENT_STATUS.has(status)) return true;
		if (status === 401 || status === 403 || status === 404) return true;
		return false;
	}
	const message =
		typeof candidate?.message === "string"
			? candidate.message
			: typeof error === "string"
				? error
				: "";
	return TRANSIENT_MESSAGE.test(message);
};

export const REMOTE_BOARD_APPLIED_EVENT = "flow:remote-board-applied";

/**
 * Why a remote board landed on this device. A `sync` merge (a peer's edit, or our own undo coming
 * back from the hub) leaves recorded history valid — the board still holds what the entries
 * describe, last writer wins. A `reset` replaced the board wholesale and discarded local edits,
 * so the entries describe a board that no longer exists.
 */
export type RemoteBoardAppliedReason = "sync" | "reset";

export interface RemoteBoardAppliedDetail {
	appId: string;
	boardId: string;
	reason?: RemoteBoardAppliedReason;
	/** The local board and merged result isolate incoming changes from edits already applied here. */
	merge?: { before: IBoard; after: IBoard };
}

export const readRemoteBoardApplied = (
	event: Event,
): RemoteBoardAppliedDetail | undefined => {
	const detail = (event as CustomEvent<Partial<RemoteBoardAppliedDetail>>)
		.detail;
	if (!detail?.appId || !detail?.boardId) return undefined;
	return {
		appId: detail.appId,
		boardId: detail.boardId,
		reason: detail.reason === "reset" ? "reset" : "sync",
		...(detail.merge?.before && detail.merge?.after
			? { merge: detail.merge }
			: {}),
	};
};

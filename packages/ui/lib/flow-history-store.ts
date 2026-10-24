import {
	readRetentionPolicy,
	retentionCutoff,
} from "./browser-storage/retention";
import {
	type HistoryEntry,
	type HistorySummary,
	type HistoryTimeline,
	REMOTE_BOARD_APPLIED_EVENT,
	appliedSeq,
	discardEntry,
	emptyTimeline,
	pushEntry,
	readRemoteBoardApplied,
	stepBack,
	stepForward,
	summarizeTimeline,
	timelineFromRows,
	withCursor,
} from "./flow-history";
import type { IGenericCommand } from "./schema";

export type HistoryDirection = "undo" | "redo";

/** How a FlowPilot receipt joins the history: on top, or by invalidating what is there. */
export type HistoryRecordMode = "append" | "invalidate";

/**
 * One atomic change to a board's persisted history. `put` and `remove` are entry rows; the cursor
 * (`appliedSeq`, `nextSeq`) is written in the same transaction so a crash can never leave the
 * cursor pointing at rows that do not exist. `clear` drops every row first.
 */
export interface HistoryWrite {
	put?: HistoryEntry[];
	remove?: number[];
	clear?: boolean;
	appliedSeq: number;
	nextSeq: number;
	/** Exactly-once marker recorded together with the entry it accounts for. */
	delivery?: string;
}

export interface HistorySnapshot {
	rows: HistoryEntry[];
	appliedSeq: number;
	nextSeq: number;
}

export interface HistoryPersistence {
	load(board: string): Promise<HistorySnapshot | undefined>;
	write(board: string, change: HistoryWrite): Promise<void>;
	hasDelivery(board: string, deliveryId: string): Promise<boolean>;
	/** Forget boards nobody edited since `before` (epoch ms). Best effort. */
	gc?(before: number): Promise<void>;
}

const EMPTY_SUMMARY: HistorySummary = {
	canUndo: false,
	canRedo: false,
	undoDepth: 0,
	redoDepth: 0,
};

const sameSummary = (a: HistorySummary, b: HistorySummary) =>
	a.canUndo === b.canUndo &&
	a.canRedo === b.canRedo &&
	a.undoDepth === b.undoDepth &&
	a.redoDepth === b.redoDepth;

interface StepFailure {
	direction: HistoryDirection;
	seq: number;
}

/**
 * The undo history of one board for this session.
 *
 * The timeline lives in memory and is the source of truth; persistence is write-behind, one small
 * transaction per change, and only ever read once (hydration). Every mutation is synchronous once
 * `ready` has settled, so callers that await `ready` observe a consistent order. Board mutations
 * and replays go through `exclusive`, which serializes them per board: an undo issued while an
 * edit is still committing waits for that edit to be recorded and then undoes *it*, never the one
 * before.
 */
export class BoardHistory {
	private timeline: HistoryTimeline = emptyTimeline();
	private summary: HistorySummary = EMPTY_SUMMARY;
	private readonly listeners = new Set<() => void>();
	private writes: Promise<void> = Promise.resolve();
	private lockTail: Promise<unknown> = Promise.resolve();
	private lastFailure: StepFailure | undefined;
	/** Deliveries recorded this session; persistence answers for earlier sessions. */
	private readonly deliveries = new Set<string>();
	private disposed = false;
	readonly ready: Promise<void>;

	constructor(
		readonly key: string,
		private readonly persistence: HistoryPersistence | undefined,
	) {
		this.ready = this.hydrate();
	}

	private async hydrate(): Promise<void> {
		if (!this.persistence) return;
		try {
			const snapshot = await this.persistence.load(this.key);
			if (!snapshot) return;
			this.commit(
				timelineFromRows(snapshot.rows, snapshot.appliedSeq, snapshot.nextSeq),
			);
		} catch (error) {
			console.warn(
				`[flow-history] Could not restore history for ${this.key}; starting empty:`,
				error,
			);
		}
	}

	private commit(timeline: HistoryTimeline): void {
		this.timeline = timeline;
		const summary = summarizeTimeline(timeline);
		if (sameSummary(summary, this.summary)) return;
		this.summary = summary;
		for (const listener of this.listeners) listener();
	}

	private persist(change: HistoryWrite): void {
		const persistence = this.persistence;
		if (!persistence) return;
		this.writes = this.writes
			.then(() => persistence.write(this.key, change))
			.catch((error) => {
				console.warn(
					`[flow-history] Could not persist history for ${this.key}:`,
					error,
				);
			});
	}

	private cursorWrite(): HistoryWrite {
		return {
			appliedSeq: appliedSeq(this.timeline),
			nextSeq: this.timeline.nextSeq,
		};
	}

	getSummary(): HistorySummary {
		return this.summary;
	}

	getTimeline(): HistoryTimeline {
		return this.timeline;
	}

	subscribe(listener: () => void): () => void {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	}

	/**
	 * Run `operation` after every earlier exclusive operation on this board has settled. Not
	 * re-entrant: an operation must not call `exclusive` on the same board again.
	 */
	exclusive<T>(operation: () => Promise<T>): Promise<T> {
		const result = this.lockTail.then(operation, operation);
		this.lockTail = result.then(
			() => undefined,
			() => undefined,
		);
		return result;
	}

	/** Record an executed batch on top of the applied entries, discarding any redo tail. */
	async record(
		commands: readonly IGenericCommand[],
		delivery?: string,
	): Promise<HistoryEntry | undefined> {
		await this.ready;
		if (commands.length === 0) return undefined;
		const pushed = pushEntry(this.timeline, commands, Date.now());
		this.lastFailure = undefined;
		this.commit(pushed.timeline);
		this.persist({
			put: [pushed.entry],
			remove: pushed.removed.map((entry) => entry.seq),
			delivery,
			...this.cursorWrite(),
		});
		return pushed.entry;
	}

	/**
	 * Record a batch at most once per `deliveryId`, even across sessions. Returns whether this call
	 * recorded it. `invalidate` acknowledges the delivery but replaces the history with nothing:
	 * the batch was applied at an unknown point in the past, so its inverse cannot sit on top.
	 */
	async recordOnce(
		commands: readonly IGenericCommand[],
		deliveryId: string,
		mode: HistoryRecordMode = "append",
	): Promise<boolean> {
		await this.ready;
		if (this.deliveries.has(deliveryId)) return false;
		if (await this.persistence?.hasDelivery(this.key, deliveryId)) {
			this.deliveries.add(deliveryId);
			return false;
		}
		if (this.deliveries.has(deliveryId)) return false;
		this.deliveries.add(deliveryId);
		if (mode === "append") {
			await this.record(commands, deliveryId);
			return true;
		}
		this.lastFailure = undefined;
		this.commit(emptyTimeline(this.timeline.nextSeq));
		this.persist({ clear: true, delivery: deliveryId, ...this.cursorWrite() });
		return true;
	}

	/**
	 * Move the cursor one step and hand back the entry to replay. The move is in memory only until
	 * `confirm` or `fail` settles it, so a crash mid-replay restores the cursor the board agrees
	 * with.
	 */
	async take(direction: HistoryDirection): Promise<HistoryEntry | undefined> {
		await this.ready;
		const step =
			direction === "undo"
				? stepBack(this.timeline)
				: stepForward(this.timeline);
		if (!step) return undefined;
		this.commit(step.timeline);
		return step.entry;
	}

	/** The replay of the entry `take` returned succeeded; the cursor position is now durable. */
	confirm(): void {
		this.lastFailure = undefined;
		this.persist(this.cursorWrite());
	}

	/**
	 * The replay of `entry` failed and the board was left as it was. The cursor moves back so the
	 * entry stays where it was. A `transient` failure (the request never reached a decision) says
	 * nothing about the entry. A second consecutive refusal of the same entry means the board no
	 * longer holds what it describes; it is dropped so history does not wedge on it forever.
	 * Returns whether the entry was dropped.
	 */
	fail(
		direction: HistoryDirection,
		entry: HistoryEntry,
		transient = false,
	): boolean {
		const restored = withCursor(
			this.timeline,
			direction === "undo"
				? this.timeline.cursor + 1
				: this.timeline.cursor - 1,
		);
		const repeated =
			this.lastFailure?.direction === direction &&
			this.lastFailure.seq === entry.seq;
		if (transient || !repeated) {
			if (!transient) this.lastFailure = { direction, seq: entry.seq };
			this.commit(restored);
			return false;
		}
		this.lastFailure = undefined;
		this.commit(discardEntry(restored, entry));
		this.persist({ remove: [entry.seq], ...this.cursorWrite() });
		return true;
	}

	/** Drop `entry` outright — its payload cannot be replayed (corrupt row). */
	discard(entry: HistoryEntry): void {
		this.lastFailure = undefined;
		this.commit(discardEntry(this.timeline, entry));
		this.persist({ remove: [entry.seq], ...this.cursorWrite() });
	}

	async clear(): Promise<void> {
		await this.ready;
		this.lastFailure = undefined;
		this.commit(emptyTimeline(this.timeline.nextSeq));
		this.persist({ clear: true, ...this.cursorWrite() });
	}

	/** Wait for every queued persistence write. */
	flush(): Promise<void> {
		return this.writes;
	}

	async dispose(): Promise<void> {
		this.disposed = true;
		await this.flush();
	}

	get isDisposed(): boolean {
		return this.disposed;
	}
}

export const boardHistoryKey = (appId: string, boardId: string) =>
	`${appId}_${boardId}`;

const IDLE_EVICTION_MS = 60_000;

/**
 * One `BoardHistory` per board for the whole session, shared by every component that edits it.
 * Histories stay resident while a board is mounted (`retain`) and are dropped a minute after the
 * last release; the next mount rehydrates from persistence.
 */
export class BoardHistoryRegistry {
	private readonly histories = new Map<string, BoardHistory>();
	private readonly retained = new Map<string, number>();
	private readonly evictions = new Map<string, ReturnType<typeof setTimeout>>();
	private gcScheduled = false;

	constructor(private persistence: HistoryPersistence | undefined) {}

	/** Swap the persistence backend (tests). Only affects histories created afterwards. */
	usePersistence(persistence: HistoryPersistence | undefined): void {
		this.persistence = persistence;
	}

	get(appId: string, boardId: string): BoardHistory {
		const key = boardHistoryKey(appId, boardId);
		const existing = this.histories.get(key);
		if (existing && !existing.isDisposed) return existing;
		const history = new BoardHistory(key, this.persistence);
		this.histories.set(key, history);
		this.scheduleGc();
		return history;
	}

	peek(appId: string, boardId: string): BoardHistory | undefined {
		const history = this.histories.get(boardHistoryKey(appId, boardId));
		return history && !history.isDisposed ? history : undefined;
	}

	retain(appId: string, boardId: string): () => void {
		const key = boardHistoryKey(appId, boardId);
		const pending = this.evictions.get(key);
		if (pending) {
			clearTimeout(pending);
			this.evictions.delete(key);
		}
		this.retained.set(key, (this.retained.get(key) ?? 0) + 1);
		this.get(appId, boardId);
		let released = false;
		return () => {
			if (released) return;
			released = true;
			const count = (this.retained.get(key) ?? 1) - 1;
			if (count > 0) {
				this.retained.set(key, count);
				return;
			}
			this.retained.delete(key);
			this.evictions.set(
				key,
				setTimeout(() => {
					this.evictions.delete(key);
					void this.evict(key);
				}, IDLE_EVICTION_MS),
			);
		};
	}

	private async evict(key: string): Promise<void> {
		if (this.retained.has(key)) return;
		const history = this.histories.get(key);
		if (!history) return;
		this.histories.delete(key);
		await history.dispose();
	}

	/**
	 * Forget a board's history whether or not it is resident. Used when the board itself was
	 * replaced (server reset, app removal): the entries describe a board that no longer exists.
	 */
	async clear(appId: string, boardId: string): Promise<void> {
		await this.clearKey(boardHistoryKey(appId, boardId));
	}

	/** `clear` for callers that only know the stored key, such as the storage settings. */
	async clearKey(key: string): Promise<void> {
		const live = this.histories.get(key);
		if (live && !live.isDisposed) {
			await live.clear();
			return;
		}
		await this.persistence?.write(key, {
			clear: true,
			appliedSeq: 0,
			nextSeq: 1,
		});
	}

	private scheduleGc(): void {
		if (this.gcScheduled || !this.persistence?.gc) return;
		this.gcScheduled = true;
		const gc = this.persistence.gc.bind(this.persistence);
		setTimeout(() => {
			const days = readRetentionPolicy().undoHistory;
			if (days === null) return;
			gc(retentionCutoff(days)).catch((error) => {
				console.warn(
					"[flow-history] History garbage collection failed:",
					error,
				);
			});
		}, 5_000);
	}

	/** Test hook: drop every resident history without touching persistence. */
	resetForTests(): void {
		for (const timer of this.evictions.values()) clearTimeout(timer);
		this.evictions.clear();
		this.retained.clear();
		this.histories.clear();
		this.gcScheduled = false;
	}
}

/**
 * A server reset replaced the board wholesale, so recorded entries describe a board that is gone.
 * Plain syncs (peer edits, our own undo returning from the hub) keep history: the board still
 * holds what the entries describe.
 */
export const remoteBoardAppliedHandler =
	(registry: Pick<BoardHistoryRegistry, "clear">) =>
	(event: Event): void => {
		const detail = readRemoteBoardApplied(event);
		if (!detail || detail.reason !== "reset") return;
		void registry.clear(detail.appId, detail.boardId).catch((error) => {
			console.warn(
				"[flow-history] Failed to clear history after a board reset:",
				error,
			);
		});
	};

export const installRemoteBoardAppliedListener = (
	registry: Pick<BoardHistoryRegistry, "clear">,
): (() => void) => {
	if (typeof window === "undefined") return () => {};
	const handler = remoteBoardAppliedHandler(registry);
	window.addEventListener(REMOTE_BOARD_APPLIED_EVENT, handler);
	return () => window.removeEventListener(REMOTE_BOARD_APPLIED_EVENT, handler);
};

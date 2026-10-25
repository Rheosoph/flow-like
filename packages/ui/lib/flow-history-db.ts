import Dexie, { type EntityTable } from "dexie";
import type { HistoryEntry } from "./flow-history";
import type {
	HistoryPersistence,
	HistorySnapshot,
	HistoryWrite,
} from "./flow-history-store";

/**
 * One row per recorded batch. `payload` is the batch as one JSON string: on desktop IndexedDB is
 * a SQLite shim whose structured-clone encoder type-walks every stored value, so a nested object
 * of node payloads costs orders of magnitude more than a string. Rows are appended and removed
 * individually — a push never rewrites the rest of the history.
 */
interface HistoryRow {
	id: string;
	board: string;
	seq: number;
	at: number;
	payload: string;
}

interface CursorRow {
	board: string;
	appliedSeq: number;
	nextSeq: number;
	updatedAt: number;
}

interface DeliveryRow {
	key: string;
	boardKey: string;
	deliveryId: string;
	createdAt: Date;
}

class UndoRedoDB extends Dexie {
	entries!: EntityTable<HistoryRow, "id">;
	cursors!: EntityTable<CursorRow, "board">;
	deliveries!: EntityTable<DeliveryRow, "key">;

	constructor() {
		super("undo-redo");
		this.version(1).stores({
			stacks: "key",
		});
		this.version(2).stores({
			stacks: "key",
			deliveries: "key, boardKey, createdAt",
		});
		this.version(3).stores({
			stacks: "key",
			meta: "key",
			deliveries: "key, boardKey, createdAt",
		});
		// v4 replaces the single stack blob with per-entry rows; the legacy blobs are dropped.
		this.version(4).stores({
			stacks: null,
			meta: null,
			entries: "id, board",
			cursors: "board, updatedAt",
			deliveries: "key, boardKey, createdAt",
		});
	}
}

let database: UndoRedoDB | undefined;

const db = (): UndoRedoDB => {
	if (!database) database = new UndoRedoDB();
	return database;
};

const SEQ_WIDTH = 12;

const SEPARATOR = "\u001f";

const rowId = (board: string, seq: number) =>
	`${board}${SEPARATOR}${seq.toString().padStart(SEQ_WIDTH, "0")}`;

/** Same shape as the v2/v3 marker keys, so receipts delivered before v4 stay recognised. */
const deliveryKey = (board: string, deliveryId: string) =>
	`${board}${SEPARATOR}${deliveryId}`;

const toEntry = (row: HistoryRow): HistoryEntry => ({
	seq: row.seq,
	at: row.at,
	payload: row.payload,
	bytes: row.payload.length,
});

export class DexieHistoryPersistence implements HistoryPersistence {
	async load(board: string): Promise<HistorySnapshot | undefined> {
		const [rows, cursor] = await Promise.all([
			db().entries.where("board").equals(board).toArray(),
			db().cursors.get(board),
		]);
		if (rows.length === 0 && !cursor) return undefined;
		return {
			rows: rows.map(toEntry),
			appliedSeq: cursor?.appliedSeq ?? 0,
			nextSeq: cursor?.nextSeq ?? 1,
		};
	}

	async write(board: string, change: HistoryWrite): Promise<void> {
		const store = db();
		await store.transaction(
			"rw",
			store.entries,
			store.cursors,
			store.deliveries,
			async () => {
				if (change.clear) {
					await store.entries.where("board").equals(board).delete();
				} else if (change.remove && change.remove.length > 0) {
					await store.entries.bulkDelete(
						change.remove.map((seq) => rowId(board, seq)),
					);
				}
				if (change.put && change.put.length > 0) {
					await store.entries.bulkPut(
						change.put.map((entry) => ({
							id: rowId(board, entry.seq),
							board,
							seq: entry.seq,
							at: entry.at,
							payload: entry.payload,
						})),
					);
				}
				await store.cursors.put({
					board,
					appliedSeq: change.appliedSeq,
					nextSeq: change.nextSeq,
					updatedAt: Date.now(),
				});
				if (change.delivery) {
					await store.deliveries.put({
						key: deliveryKey(board, change.delivery),
						boardKey: board,
						deliveryId: change.delivery,
						createdAt: new Date(),
					});
				}
			},
		);
	}

	async hasDelivery(board: string, deliveryId: string): Promise<boolean> {
		return (
			(await db().deliveries.get(deliveryKey(board, deliveryId))) !== undefined
		);
	}

	async gc(before: number): Promise<void> {
		await pruneBoardHistories(before);
	}
}

export const createHistoryPersistence = (): HistoryPersistence | undefined =>
	typeof indexedDB === "undefined" ? undefined : new DexieHistoryPersistence();

/**
 * Drop the entries of boards untouched since `before` and return how many boards lost theirs.
 * Delivery markers are kept: they are tiny, and a late receipt replay must still be recognised.
 */
export async function pruneBoardHistories(before: number): Promise<number> {
	const store = db();
	const stale = await store.cursors.where("updatedAt").below(before).toArray();
	if (stale.length === 0) return 0;
	await store.transaction("rw", store.entries, store.cursors, async () => {
		for (const cursor of stale) {
			await store.entries.where("board").equals(cursor.board).delete();
			await store.cursors.delete(cursor.board);
		}
	});
	return stale.length;
}

export interface BoardHistorySummary {
	/** `boardHistoryKey(appId, boardId)` */
	key: string;
	entries: number;
	bytes: number;
	updatedAt?: number;
}

/** One pass over the entry rows, so memory stays at one payload regardless of history size. */
export async function summarizeBoardHistories(): Promise<
	BoardHistorySummary[]
> {
	const store = db();
	const summaries = new Map<string, BoardHistorySummary>();
	await store.entries.each((row) => {
		const summary = summaries.get(row.board) ?? {
			key: row.board,
			entries: 0,
			bytes: 0,
		};
		summary.entries += 1;
		summary.bytes += row.payload.length + row.id.length + row.board.length + 16;
		summaries.set(row.board, summary);
	});
	for (const cursor of await store.cursors.toArray()) {
		const summary = summaries.get(cursor.board);
		if (summary) summary.updatedAt = cursor.updatedAt;
	}
	return [...summaries.values()];
}

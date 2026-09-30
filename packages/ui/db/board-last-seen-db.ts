import Dexie, { type EntityTable } from "dexie";
import type {
	PackedSnapshot,
	SnapshotEncoding,
} from "../lib/board-diff/snapshot-codec";

export interface IBoardLastSeenRecord {
	/** `${userKey}:${appId}:${boardId}` */
	id: string;
	appId: string;
	boardId: string;
	/**
	 * The board as the user last had it on screen, gzip-compressed and base64-encoded
	 * (see `SnapshotEncoding`). Decoding and parsing happen in the snapshot worker, so a
	 * read never parses a board on the main thread.
	 */
	payload: string;
	encoding: SnapshotEncoding;
	seenAt: number;
	updatedAt: number;
}

const boardLastSeenDb = new Dexie("Board-Last-Seen") as Dexie & {
	seen: EntityTable<IBoardLastSeenRecord, "id">;
};

boardLastSeenDb.version(1).stores({
	seen: "&id, appId, boardId, updatedAt",
});
// v1 kept the raw board JSON in `board`. Snapshots are only a baseline for the next
// notice, so v1 rows are dropped rather than re-encoded on the main thread.
boardLastSeenDb
	.version(2)
	.stores({ seen: "&id, appId, boardId, updatedAt" })
	.upgrade((tx) => tx.table("seen").clear());

export { boardLastSeenDb };

export function boardLastSeenKey(
	userKey: string,
	appId: string,
	boardId: string,
) {
	return `${userKey}:${appId}:${boardId}`;
}

export interface IStoredBoardSnapshot extends PackedSnapshot {
	seenAt: number;
}

export async function readBoardLastSeen(
	userKey: string,
	appId: string,
	boardId: string,
): Promise<IStoredBoardSnapshot | undefined> {
	const record = await boardLastSeenDb.seen.get(
		boardLastSeenKey(userKey, appId, boardId),
	);
	if (!record || typeof record.payload !== "string") return undefined;
	return {
		payload: record.payload,
		encoding: record.encoding,
		seenAt: record.seenAt,
	};
}

export async function writeBoardLastSeen(
	userKey: string,
	appId: string,
	boardId: string,
	snapshot: PackedSnapshot,
	seenAt = Date.now(),
): Promise<void> {
	await boardLastSeenDb.seen.put({
		id: boardLastSeenKey(userKey, appId, boardId),
		appId,
		boardId,
		payload: snapshot.payload,
		encoding: snapshot.encoding,
		seenAt,
		updatedAt: Date.now(),
	});
}

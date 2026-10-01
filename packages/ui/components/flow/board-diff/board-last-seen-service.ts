import {
	type IStoredBoardSnapshot,
	readBoardLastSeen,
	writeBoardLastSeen,
} from "../../../db/board-last-seen-db";
import { serializeBoard } from "../../../lib/board-diff/board-json";
import { snapshotRequest } from "../../../lib/board-diff/snapshot-client";
import type { PackedSnapshot } from "../../../lib/board-diff/snapshot-codec";
import type {
	SnapshotRequest,
	SnapshotResponse,
} from "../../../lib/board-diff/snapshot-engine";
import type { IBoardDiff } from "../../../lib/board-diff/types";
import type { IBoard } from "../../../lib/schema/flow/board";

export interface LastSeenIds {
	userKey: string;
	appId: string;
	boardId: string;
}

export interface LastSeenStore {
	read(
		userKey: string,
		appId: string,
		boardId: string,
	): Promise<IStoredBoardSnapshot | undefined>;
	write(
		userKey: string,
		appId: string,
		boardId: string,
		snapshot: PackedSnapshot,
	): Promise<void>;
}

export type LastSeenOpenResult =
	| { kind: "first" }
	| { kind: "same"; seenAt: number }
	| { kind: "news"; seenAt: number; base: IBoard; diff: IBoardDiff };

export interface BoardLastSeenService {
	/** Reads the stored snapshot and compares it with `board` in the snapshot worker. */
	open(ids: LastSeenIds, board: IBoard): Promise<LastSeenOpenResult>;
	/** Compares `board` with the snapshot read at `open`. */
	compare(
		ids: LastSeenIds,
		board: IBoard,
		withBase: boolean,
	): Promise<{ diff?: IBoardDiff; base?: IBoard }>;
	/** Stores `board` unless the stored snapshot already has the same text. Never blocks. */
	save(ids: LastSeenIds, board: IBoard): Promise<void>;
	/** Page is unloading: serialize synchronously and start the write before it goes. */
	saveNow(ids: LastSeenIds, board: IBoard): void;
	forget(ids: LastSeenIds): void;
}

const keyOf = ({ userKey, appId, boardId }: LastSeenIds) =>
	`${userKey}:${appId}:${boardId}`;

type Request = (request: SnapshotRequest) => Promise<SnapshotResponse>;

export function createBoardLastSeenService(
	store: LastSeenStore = { read: readBoardLastSeen, write: writeBoardLastSeen },
	request: Request = snapshotRequest,
): BoardLastSeenService {
	const warn = (what: string, ids: LastSeenIds, error: unknown) =>
		console.warn(
			`[board-last-seen] ${what} failed for board ${ids.boardId} in app ${ids.appId}`,
			error,
		);

	return {
		async open(ids, board) {
			const [stored, current] = await Promise.all([
				store.read(ids.userKey, ids.appId, ids.boardId).catch((error) => {
					warn("Reading the last seen snapshot", ids, error);
					return undefined;
				}),
				serializeBoard(board),
			]);
			const response = await request({
				type: "open",
				key: keyOf(ids),
				stored,
				current,
			});
			if (response.type !== "open" || response.first || !stored)
				return { kind: "first" };
			if (!response.diff || !response.base)
				return { kind: "same", seenAt: stored.seenAt };
			return {
				kind: "news",
				seenAt: stored.seenAt,
				// The one parse on the main thread, and only when a notice is about to show.
				base: JSON.parse(response.base) as IBoard,
				diff: response.diff,
			};
		},

		async compare(ids, board, withBase) {
			const response = await request({
				type: "compare",
				key: keyOf(ids),
				current: await serializeBoard(board),
				withBase,
			}).catch((error) => {
				warn("Comparing with the last seen snapshot", ids, error);
				return undefined;
			});
			if (response?.type !== "compare" || !response.diff) return {};
			return {
				diff: response.diff,
				base: response.base ? (JSON.parse(response.base) as IBoard) : undefined,
			};
		},

		async save(ids, board) {
			try {
				const response = await request({
					type: "pack",
					key: keyOf(ids),
					current: await serializeBoard(board),
				});
				if (response.type !== "pack" || !response.packed) return;
				await store
					.write(ids.userKey, ids.appId, ids.boardId, response.packed)
					.catch(async (error) => {
						await request({ type: "unmark", key: keyOf(ids) });
						throw error;
					});
			} catch (error) {
				warn("Storing the last seen snapshot", ids, error);
			}
		},

		saveNow(ids, board) {
			store
				.write(ids.userKey, ids.appId, ids.boardId, {
					payload: JSON.stringify(board),
					encoding: "json",
				})
				.catch((error) => warn("Storing the snapshot on unload", ids, error));
		},

		forget(ids) {
			void request({ type: "forget", key: keyOf(ids) }).catch(() => undefined);
		},
	};
}

let defaultService: BoardLastSeenService | undefined;
export function boardLastSeenService(): BoardLastSeenService {
	defaultService ??= createBoardLastSeenService();
	return defaultService;
}

import { i18n as i18next } from "@flow-like/locales";
import Dexie from "dexie";
import { HistoryIcon } from "lucide-react";
import type { IGenericCommand } from "../../lib";
import {
	type IHistoryStacks,
	REMOTE_BOARD_APPLIED_EVENT,
	emptyStacks,
	pushBatch,
	remoteBoardAppliedHandler,
	rollbackRedoBatch,
	rollbackUndoBatch,
	takeRedo,
	takeUndo,
} from "../../lib/flow-history-stacks";
import type { BoardEditReceiptHistoryMode } from "../../lib/flowpilot/board-edit-job-delivery";
import { toastWarning } from "../../lib/messages";

/**
 * The stacks are persisted as one JSON string. On desktop, IndexedDB is a
 * SQLite shim whose structured-clone encoder type-walks every stored value —
 * several full passes per write — so a nested object holding up to
 * MAX_STACK_SIZE batches of full node payloads froze the UI for hundreds of
 * milliseconds after every board edit. A string is encoded in one cheap pass.
 * Rows written before `payload` existed still carry the inline legacy shape.
 */
interface IStackItem extends Partial<IHistoryStacks> {
	key: string;
	payload?: string;
}

/** Kept apart from the stacks so re-stamping never rewrites the whole history. */
interface IStackMeta {
	key: string;
	boardStamp?: string;
}

interface IHistoryDelivery {
	key: string;
	boardKey: string;
	deliveryId: string;
	createdAt: Date;
}

class UndoRedoDB extends Dexie {
	stacks!: Dexie.Table<IStackItem, string>;
	meta!: Dexie.Table<IStackMeta, string>;
	deliveries!: Dexie.Table<IHistoryDelivery, string>;

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
	}
}

const db = new UndoRedoDB();

const historyKey = (appId: string, boardId: string) => `${appId}_${boardId}`;

const deleteStacks = async (key: string) => {
	await Promise.all([db.stacks.delete(key), db.meta.delete(key)]);
};

export const clearBoardHistory = async (appId: string, boardId: string) => {
	await deleteStacks(historyKey(appId, boardId));
};

// Remote board applies must invalidate persisted stacks even when the board
// is not mounted anywhere — the entries were recorded against a board state
// that no longer exists.
if (typeof window !== "undefined") {
	window.addEventListener(
		REMOTE_BOARD_APPLIED_EVENT,
		remoteBoardAppliedHandler(clearBoardHistory),
	);
}

const decodeStacks = (data: IStackItem | undefined): IHistoryStacks => {
	if (!data) return emptyStacks();
	const decoded: Partial<IHistoryStacks> = data.payload
		? JSON.parse(data.payload)
		: data;
	return {
		undoStack: decoded.undoStack ?? [],
		redoStack: decoded.redoStack ?? [],
		boardStamp: decoded.boardStamp,
		deliveryIds: decoded.deliveryIds ?? [],
	};
};

const readStacks = async (key: string): Promise<IHistoryStacks> => {
	const [data, meta] = await Promise.all([
		db.stacks.get(key),
		db.meta.get(key),
	]);
	const stacks = decodeStacks(data);
	if (!meta && stacks.boardStamp) {
		// Legacy rows carried the stamp inline; move it once so the next write can drop it.
		await db.meta.put({ key, boardStamp: stacks.boardStamp });
	}
	return { ...stacks, boardStamp: meta?.boardStamp ?? stacks.boardStamp };
};

const writeStacks = async (key: string, stacks: IHistoryStacks) => {
	const { boardStamp: _boardStamp, ...rest } = stacks;
	await db.stacks.put({ key, payload: JSON.stringify(rest) });
};

const toastStaleHistory = (action: "Undo" | "Redo") => {
	toastWarning(
		i18next.t('actionHistoryWasRecordedAgainstAnOlderVersionOfThisBoardAndHasBeenCleared', '{{action}} history was recorded against an older version of this board and has been cleared', { action }),
		<HistoryIcon className="w-4 h-4" />,
	);
};

export const useUndoRedo = (appId: string, boardId: string) => {
	const key = historyKey(appId, boardId);

	const clearHistory = async () => {
		await deleteStacks(key);
	};

	const pushCommand = async (command: IGenericCommand, append = false) => {
		await db.transaction("rw", db.stacks, db.meta, async () => {
			const stacks = await readStacks(key);
			await writeStacks(key, pushBatch(stacks, [command], append));
		});
	};

	const pushCommands = async (commands: IGenericCommand[]) => {
		await db.transaction("rw", db.stacks, db.meta, async () => {
			const stacks = await readStacks(key);
			await writeStacks(key, pushBatch(stacks, commands));
		});
	};

	const pushCommandsOnce = async (
		commands: IGenericCommand[],
		deliveryId: string,
		historyMode: BoardEditReceiptHistoryMode = "append",
	) => {
		const deliveryKey = `${key}\u001f${deliveryId}`;
		await db.transaction("rw", db.stacks, db.meta, db.deliveries, async () => {
			if (await db.deliveries.get(deliveryKey)) return;
			const stacks = await readStacks(key);
			// Migrate the former stack-local marker without duplicating its history batch.
			if (stacks.deliveryIds?.includes(deliveryId)) {
				await db.deliveries.put({
					key: deliveryKey,
					boardKey: key,
					deliveryId,
					createdAt: new Date(),
				});
				return;
			}
			if (historyMode === "append") {
				const next = pushBatch(stacks, commands);
				await writeStacks(key, next);
			} else {
				// A rehydrated native apply may predate newer user edits. Recording its inverse batch
				// on top would make Undo replay history out of order, so atomically invalidate the
				// stack while retaining the exactly-once delivery marker.
				await deleteStacks(key);
			}
			await db.deliveries.put({
				key: deliveryKey,
				boardKey: key,
				deliveryId,
				createdAt: new Date(),
			});
		});
	};

	const stampHistory = async (stamp?: string) => {
		await db.transaction("rw", db.stacks, db.meta, async () => {
			if ((await db.stacks.where("key").equals(key).count()) === 0) return;
			await db.meta.put({ key, boardStamp: stamp });
		});
	};

	const undo = async (currentStamp?: string) => {
		const result = await db.transaction("rw", db.stacks, db.meta, async () => {
			const stacks = await readStacks(key);
			const taken = takeUndo(stacks, currentStamp);
			if (taken.stale) {
				await deleteStacks(key);
				return taken;
			}
			if (taken.batch) {
				await writeStacks(key, taken.stacks);
			}
			return taken;
		});

		if (result.stale) {
			toastStaleHistory("Undo");
			return null;
		}
		return result.batch;
	};

	const redo = async (currentStamp?: string) => {
		const result = await db.transaction("rw", db.stacks, db.meta, async () => {
			const stacks = await readStacks(key);
			const taken = takeRedo(stacks, currentStamp);
			if (taken.stale) {
				await deleteStacks(key);
				return taken;
			}
			if (taken.batch) {
				await writeStacks(key, taken.stacks);
			}
			return taken;
		});

		if (result.stale) {
			toastStaleHistory("Redo");
			return null;
		}
		return result.batch;
	};

	const rollbackUndo = async (commands: IGenericCommand[]) => {
		await db.transaction("rw", db.stacks, db.meta, async () => {
			const stacks = await readStacks(key);
			const rolledBack = rollbackUndoBatch(stacks, commands);
			if (rolledBack === stacks) return;
			await writeStacks(key, rolledBack);
		});
	};

	const rollbackRedo = async (commands: IGenericCommand[]) => {
		await db.transaction("rw", db.stacks, db.meta, async () => {
			const stacks = await readStacks(key);
			const rolledBack = rollbackRedoBatch(stacks, commands);
			if (rolledBack === stacks) return;
			await writeStacks(key, rolledBack);
		});
	};

	return {
		pushCommand,
		pushCommands,
		pushCommandsOnce,
		undo,
		redo,
		rollbackUndo,
		rollbackRedo,
		clearHistory,
		stampHistory,
	};
};

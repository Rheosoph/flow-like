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
	stampStacks,
	takeRedo,
	takeUndo,
} from "../../lib/flow-history-stacks";
import type { BoardEditReceiptHistoryMode } from "../../lib/flowpilot/board-edit-job-delivery";
import { toastWarning } from "../../lib/messages";

interface IStackItem extends IHistoryStacks {
	key: string;
}

interface IHistoryDelivery {
	key: string;
	boardKey: string;
	deliveryId: string;
	createdAt: Date;
}

class UndoRedoDB extends Dexie {
	stacks!: Dexie.Table<IStackItem, string>;
	deliveries!: Dexie.Table<IHistoryDelivery, string>;

	constructor() {
		super("undo-redo");
		this.version(1).stores({
			stacks: "key",
		});
		this.version(2).stores({
			stacks: "key",
			deliveries: i18next.t('keyBoardkeyCreatedat', 'key, boardKey, createdAt'),
		});
	}
}

const db = new UndoRedoDB();

const historyKey = (appId: string, boardId: string) => `${appId}_${boardId}`;

export const clearBoardHistory = async (appId: string, boardId: string) => {
	await db.stacks.delete(historyKey(appId, boardId));
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

const readStacks = (data: IStackItem | undefined): IHistoryStacks =>
	data
		? {
				undoStack: data.undoStack ?? [],
				redoStack: data.redoStack ?? [],
				boardStamp: data.boardStamp,
				deliveryIds: data.deliveryIds ?? [],
			}
		: emptyStacks();

const writeStacks = async (key: string, stacks: IHistoryStacks) => {
	await db.stacks.put({ key, ...stacks });
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
		await db.stacks.delete(key);
	};

	const pushCommand = async (command: IGenericCommand, append = false) => {
		await db.transaction("rw", db.stacks, async () => {
			const stacks = readStacks(await db.stacks.get(key));
			await writeStacks(key, pushBatch(stacks, [command], append));
		});
	};

	const pushCommands = async (commands: IGenericCommand[]) => {
		await db.transaction("rw", db.stacks, async () => {
			const stacks = readStacks(await db.stacks.get(key));
			await writeStacks(key, pushBatch(stacks, commands));
		});
	};

	const pushCommandsOnce = async (
		commands: IGenericCommand[],
		deliveryId: string,
		historyMode: BoardEditReceiptHistoryMode = "append",
	) => {
		const deliveryKey = `${key}${deliveryId}`;
		await db.transaction("rw", db.stacks, db.deliveries, async () => {
			if (await db.deliveries.get(deliveryKey)) return;
			const data = await db.stacks.get(key);
			const stacks = readStacks(data);
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
				await db.stacks.delete(key);
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
		await db.transaction("rw", db.stacks, async () => {
			const data = await db.stacks.get(key);
			if (!data) return;
			await writeStacks(key, stampStacks(readStacks(data), stamp));
		});
	};

	const undo = async (currentStamp?: string) => {
		const result = await db.transaction("rw", db.stacks, async () => {
			const stacks = readStacks(await db.stacks.get(key));
			const taken = takeUndo(stacks, currentStamp);
			if (taken.stale) {
				await db.stacks.delete(key);
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
		const result = await db.transaction("rw", db.stacks, async () => {
			const stacks = readStacks(await db.stacks.get(key));
			const taken = takeRedo(stacks, currentStamp);
			if (taken.stale) {
				await db.stacks.delete(key);
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
		await db.transaction("rw", db.stacks, async () => {
			const stacks = readStacks(await db.stacks.get(key));
			const rolledBack = rollbackUndoBatch(stacks, commands);
			if (rolledBack === stacks) return;
			await writeStacks(key, rolledBack);
		});
	};

	const rollbackRedo = async (commands: IGenericCommand[]) => {
		await db.transaction("rw", db.stacks, async () => {
			const stacks = readStacks(await db.stacks.get(key));
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

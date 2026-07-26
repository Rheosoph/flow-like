import type { IGenericCommand } from "./schema";

export interface IHistoryStacks {
	undoStack: IGenericCommand[][];
	redoStack: IGenericCommand[][];
	/**
	 * Fingerprint of the board state the stacks were last known to match.
	 * `undefined` means unknown (legacy records or unstamped push paths) and
	 * disables staleness enforcement until the next stamped operation.
	 */
	boardStamp?: string;
	/** Legacy stack-local receipt ids; new durable markers live in a separate IndexedDB table. */
	deliveryIds?: string[];
}

export interface HistoryTakeResult {
	stacks: IHistoryStacks;
	batch: IGenericCommand[] | null;
	stale: boolean;
}

interface BoardStampSource {
	updated_at?: {
		secs_since_epoch?: number;
		nanos_since_epoch?: number;
	} | null;
}

export const MAX_STACK_SIZE = 100;

export const boardFingerprint = (
	board?: BoardStampSource | null,
): string | undefined => {
	const updatedAt = board?.updated_at;
	if (!updatedAt) return undefined;
	const secs = updatedAt.secs_since_epoch ?? 0;
	const nanos = updatedAt.nanos_since_epoch ?? 0;
	if (secs === 0 && nanos === 0) return undefined;
	return `${secs}:${nanos}`;
};

export const emptyStacks = (): IHistoryStacks => ({
	undoStack: [],
	redoStack: [],
	deliveryIds: [],
});

export const isHistoryStale = (
	stacks: Pick<IHistoryStacks, "boardStamp">,
	currentStamp: string | undefined,
): boolean =>
	Boolean(
		stacks.boardStamp && currentStamp && stacks.boardStamp !== currentStamp,
	);

const capUndoStack = (stack: IGenericCommand[][]) =>
	stack.length > MAX_STACK_SIZE ? stack.slice(1) : stack;

const capRedoStack = (stack: IGenericCommand[][]) =>
	stack.length > MAX_STACK_SIZE ? stack.slice(0, MAX_STACK_SIZE) : stack;

export const pushBatch = (
	stacks: IHistoryStacks,
	commands: IGenericCommand[],
	append = false,
): IHistoryStacks => {
	const undoStack = stacks.undoStack;
	let newUndoStack: IGenericCommand[][];

	if (append && undoStack.length > 0) {
		const lastBatch = undoStack[undoStack.length - 1];
		newUndoStack = [...undoStack.slice(0, -1), [...lastBatch, ...commands]];
	} else {
		newUndoStack = [...undoStack, commands];
	}

	// The board advanced past the last stamped state; the new fingerprint is
	// unknown until the caller re-stamps after refetching the board.
	return {
		undoStack: capUndoStack(newUndoStack),
		redoStack: [],
		deliveryIds: stacks.deliveryIds,
	};
};

export const stampStacks = (
	stacks: IHistoryStacks,
	stamp: string | undefined,
): IHistoryStacks => ({ ...stacks, boardStamp: stamp });

export const takeUndo = (
	stacks: IHistoryStacks,
	currentStamp: string | undefined,
): HistoryTakeResult => {
	if (isHistoryStale(stacks, currentStamp)) {
		return { stacks: emptyStacks(), batch: null, stale: true };
	}
	if (stacks.undoStack.length === 0) {
		return { stacks, batch: null, stale: false };
	}

	const batch = stacks.undoStack[stacks.undoStack.length - 1];
	return {
		stacks: {
			...stacks,
			undoStack: stacks.undoStack.slice(0, -1),
			redoStack: capRedoStack([batch, ...stacks.redoStack]),
		},
		batch,
		stale: false,
	};
};

export const takeRedo = (
	stacks: IHistoryStacks,
	currentStamp: string | undefined,
): HistoryTakeResult => {
	if (isHistoryStale(stacks, currentStamp)) {
		return { stacks: emptyStacks(), batch: null, stale: true };
	}
	if (stacks.redoStack.length === 0) {
		return { stacks, batch: null, stale: false };
	}

	const batch = stacks.redoStack[0];
	return {
		stacks: {
			...stacks,
			undoStack: capUndoStack([...stacks.undoStack, batch]),
			redoStack: stacks.redoStack.slice(1),
		},
		batch,
		stale: false,
	};
};

const serializeBatch = (commands: IGenericCommand[]) =>
	JSON.stringify(commands);

export const rollbackUndoBatch = (
	stacks: IHistoryStacks,
	commands: IGenericCommand[],
): IHistoryStacks => {
	const target = serializeBatch(commands);
	const rollbackIndex = stacks.redoStack.findIndex(
		(batch) => serializeBatch(batch) === target,
	);
	if (rollbackIndex === -1) return stacks;

	const rollbackBatch = stacks.redoStack[rollbackIndex];
	return {
		...stacks,
		undoStack: capUndoStack([...stacks.undoStack, rollbackBatch]),
		redoStack: stacks.redoStack.filter((_, index) => index !== rollbackIndex),
	};
};

export const rollbackRedoBatch = (
	stacks: IHistoryStacks,
	commands: IGenericCommand[],
): IHistoryStacks => {
	const target = serializeBatch(commands);
	const rollbackIndex = stacks.undoStack.findIndex(
		(batch) => serializeBatch(batch) === target,
	);
	if (rollbackIndex === -1) return stacks;

	const rollbackBatch = stacks.undoStack[rollbackIndex];
	return {
		...stacks,
		undoStack: stacks.undoStack.filter((_, index) => index !== rollbackIndex),
		redoStack: capRedoStack([rollbackBatch, ...stacks.redoStack]),
	};
};

export const REMOTE_BOARD_APPLIED_EVENT = "flow:remote-board-applied";

/**
 * Handles a remote-board-applied notification independent of any mounted
 * canvas: history recorded against the pre-apply board must not survive.
 */
export const remoteBoardAppliedHandler =
	(clear: (appId: string, boardId: string) => Promise<void>) =>
	(event: Event): void => {
		const detail = (event as CustomEvent<{ appId?: string; boardId?: string }>)
			.detail;
		if (!detail?.appId || !detail?.boardId) return;
		void clear(detail.appId, detail.boardId).catch((error) => {
			console.warn(
				"[flow-history] Failed to clear undo history after remote board apply:",
				error,
			);
		});
	};

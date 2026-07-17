import { describe, expect, test } from "bun:test";
import {
	MAX_STACK_SIZE,
	boardFingerprint,
	emptyStacks,
	isHistoryStale,
	pushBatch,
	remoteBoardAppliedHandler,
	rollbackRedoBatch,
	rollbackUndoBatch,
	stampStacks,
	takeRedo,
	takeUndo,
} from "./flow-history-stacks";
import type { IGenericCommand } from "./schema";

const command = (id: string): IGenericCommand =>
	({ command_type: "MoveNode", node_id: id }) as unknown as IGenericCommand;

const stampedStacks = (stamp: string, batches: IGenericCommand[][]) => {
	let stacks = emptyStacks();
	for (const batch of batches) {
		stacks = pushBatch(stacks, batch);
	}
	return stampStacks(stacks, stamp);
};

describe("boardFingerprint", () => {
	test("derives a stable stamp from updated_at", () => {
		const board = {
			updated_at: { secs_since_epoch: 1720000000, nanos_since_epoch: 42 },
		};
		expect(boardFingerprint(board)).toBe("1720000000:42");
		expect(boardFingerprint(board)).toBe(boardFingerprint({ ...board }));
	});

	test("returns undefined when updated_at is missing or zeroed", () => {
		expect(boardFingerprint(undefined)).toBeUndefined();
		expect(boardFingerprint(null)).toBeUndefined();
		expect(boardFingerprint({})).toBeUndefined();
		expect(
			boardFingerprint({
				updated_at: { secs_since_epoch: 0, nanos_since_epoch: 0 },
			}),
		).toBeUndefined();
	});
});

describe("stale history detection (defect: stacks replayable after board changed underneath)", () => {
	test("undo against a diverged board refuses and clears instead of replaying", () => {
		const stacks = stampedStacks("100:0", [[command("a")], [command("b")]]);

		const result = takeUndo(stacks, "200:0");

		expect(result.stale).toBe(true);
		expect(result.batch).toBeNull();
		expect(result.stacks.undoStack).toEqual([]);
		expect(result.stacks.redoStack).toEqual([]);
	});

	test("redo against a diverged board refuses and clears", () => {
		let stacks = stampedStacks("100:0", [[command("a")]]);
		const undone = takeUndo(stacks, "100:0");
		stacks = stampStacks(undone.stacks, "100:0");

		const result = takeRedo(stacks, "300:0");

		expect(result.stale).toBe(true);
		expect(result.batch).toBeNull();
		expect(result.stacks.undoStack).toEqual([]);
		expect(result.stacks.redoStack).toEqual([]);
	});

	test("undo with a matching stamp replays normally", () => {
		const stacks = stampedStacks("100:0", [[command("a")], [command("b")]]);

		const result = takeUndo(stacks, "100:0");

		expect(result.stale).toBe(false);
		expect(result.batch).toEqual([command("b")]);
		expect(result.stacks.undoStack).toEqual([[command("a")]]);
		expect(result.stacks.redoStack).toEqual([[command("b")]]);
	});

	test("missing stamps stay permissive (legacy records, unstamped push paths)", () => {
		const unstamped = stampedStacks("100:0", [[command("a")]]);
		const legacy = { ...unstamped, boardStamp: undefined };

		expect(isHistoryStale(legacy, "999:0")).toBe(false);
		expect(takeUndo(legacy, "999:0").batch).toEqual([command("a")]);
		expect(isHistoryStale(unstamped, undefined)).toBe(false);
		expect(takeUndo(unstamped, undefined).batch).toEqual([command("a")]);
	});

	test("push invalidates the previous stamp until the caller re-stamps", () => {
		const stacks = stampedStacks("100:0", [[command("a")]]);
		const pushed = pushBatch(stacks, [command("b")]);

		expect(pushed.boardStamp).toBeUndefined();
		expect(stampStacks(pushed, "200:0").boardStamp).toBe("200:0");
	});
});

describe("stack mechanics", () => {
	test("append merges into the last batch", () => {
		let stacks = pushBatch(emptyStacks(), [command("a")]);
		stacks = pushBatch(stacks, [command("b")], true);

		expect(stacks.undoStack).toEqual([[command("a"), command("b")]]);
	});

	test("append without a prior batch starts a new one", () => {
		const stacks = pushBatch(emptyStacks(), [command("a")], true);
		expect(stacks.undoStack).toEqual([[command("a")]]);
	});

	test("push clears the redo stack and caps the undo stack", () => {
		let stacks = emptyStacks();
		for (let i = 0; i < MAX_STACK_SIZE + 5; i++) {
			stacks = pushBatch(stacks, [command(`c${i}`)]);
		}
		expect(stacks.undoStack.length).toBe(MAX_STACK_SIZE);
		expect(stacks.undoStack[0]).toEqual([command("c5")]);

		const undone = takeUndo(stacks, undefined);
		expect(undone.stacks.redoStack.length).toBe(1);
		const pushed = pushBatch(undone.stacks, [command("new")]);
		expect(pushed.redoStack).toEqual([]);
	});

	test("takeUndo/takeRedo on empty stacks return null without mutation", () => {
		const stacks = emptyStacks();
		expect(takeUndo(stacks, undefined)).toEqual({
			stacks,
			batch: null,
			stale: false,
		});
		expect(takeRedo(stacks, undefined)).toEqual({
			stacks,
			batch: null,
			stale: false,
		});
	});

	test("rollbackUndoBatch moves a failed batch back onto the undo stack", () => {
		const stacks = stampedStacks("100:0", [[command("a")], [command("b")]]);
		const undone = takeUndo(stacks, "100:0");
		expect(undone.batch).toEqual([command("b")]);

		const rolledBack = rollbackUndoBatch(undone.stacks, [command("b")]);
		expect(rolledBack.undoStack).toEqual([[command("a")], [command("b")]]);
		expect(rolledBack.redoStack).toEqual([]);
	});

	test("rollbackRedoBatch moves a failed batch back onto the redo stack", () => {
		let stacks = stampedStacks("100:0", [[command("a")]]);
		const undone = takeUndo(stacks, "100:0");
		stacks = undone.stacks;
		const redone = takeRedo(stacks, undefined);
		expect(redone.batch).toEqual([command("a")]);

		const rolledBack = rollbackRedoBatch(redone.stacks, [command("a")]);
		expect(rolledBack.undoStack).toEqual([]);
		expect(rolledBack.redoStack).toEqual([[command("a")]]);
	});

	test("rollback with an unknown batch is a no-op", () => {
		const stacks = stampedStacks("100:0", [[command("a")]]);
		expect(rollbackUndoBatch(stacks, [command("zz")])).toEqual(stacks);
		expect(rollbackRedoBatch(stacks, [command("zz")])).toEqual(stacks);
	});
});

describe("remote board applied handler (defect: background applies left durable stale stacks)", () => {
	test("clears persisted history for the applied board without requiring a mounted canvas", () => {
		const cleared: Array<[string, string]> = [];
		const handler = remoteBoardAppliedHandler(async (appId, boardId) => {
			cleared.push([appId, boardId]);
		});

		handler(
			new CustomEvent("flow:remote-board-applied", {
				detail: { appId: "app-1", boardId: "board-1" },
			}),
		);

		expect(cleared).toEqual([["app-1", "board-1"]]);
	});

	test("ignores malformed events", () => {
		const cleared: Array<[string, string]> = [];
		const handler = remoteBoardAppliedHandler(async (appId, boardId) => {
			cleared.push([appId, boardId]);
		});

		handler(new CustomEvent("flow:remote-board-applied", { detail: {} }));
		handler(new CustomEvent("flow:remote-board-applied"));
		handler(new Event("flow:remote-board-applied"));

		expect(cleared).toEqual([]);
	});

	test("swallows clear failures so the event loop never sees an unhandled rejection", async () => {
		const handler = remoteBoardAppliedHandler(async () => {
			throw new Error("indexeddb unavailable");
		});

		handler(
			new CustomEvent("flow:remote-board-applied", {
				detail: { appId: "app-1", boardId: "board-1" },
			}),
		);

		await new Promise((resolve) => setTimeout(resolve, 0));
	});
});

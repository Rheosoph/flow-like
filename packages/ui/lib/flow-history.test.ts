import { describe, expect, test } from "bun:test";
import {
	type HistoryEntry,
	type HistoryTimeline,
	appliedSeq,
	decodeEntry,
	discardEntry,
	emptyTimeline,
	isTransientReplayError,
	pushEntry,
	readRemoteBoardApplied,
	stepBack,
	stepForward,
	summarizeTimeline,
	timelineFromRows,
	withCursor,
} from "./flow-history";
import type { IGenericCommand } from "./schema";
import type { IBoard } from "./schema/flow/board";

const command = (id: string): IGenericCommand =>
	({ command_type: "MoveNode", node_id: id }) as unknown as IGenericCommand;

const seqs = (timeline: HistoryTimeline) =>
	timeline.entries.map((entry) => entry.seq);

const must = <T>(value: T | undefined): T => {
	if (value === undefined) throw new Error("expected a value");
	return value;
};

const build = (...batches: string[][]): HistoryTimeline => {
	let timeline = emptyTimeline();
	for (const batch of batches) {
		timeline = pushEntry(timeline, batch.map(command), 1).timeline;
	}
	return timeline;
};

describe("timeline mechanics", () => {
	test("push appends, moves the cursor to the end and hands the entry back", () => {
		const pushed = pushEntry(emptyTimeline(), [command("a")], 42);
		expect(pushed.entry.seq).toBe(1);
		expect(pushed.entry.at).toBe(42);
		expect(decodeEntry(pushed.entry)).toEqual([command("a")]);
		expect(pushed.timeline.cursor).toBe(1);
		expect(pushed.timeline.nextSeq).toBe(2);
		expect(pushed.removed).toEqual([]);
	});

	test("undo and redo walk the cursor without touching the entries", () => {
		const timeline = build(["a"], ["b"], ["c"]);
		const undone = must(stepBack(timeline));
		expect(decodeEntry(undone.entry)).toEqual([command("c")]);
		expect(undone.timeline.cursor).toBe(2);
		expect(seqs(undone.timeline)).toEqual([1, 2, 3]);

		const redone = must(stepForward(undone.timeline));
		expect(decodeEntry(redone.entry)).toEqual([command("c")]);
		expect(redone.timeline.cursor).toBe(3);
	});

	test("undo at the start and redo at the end are no-ops", () => {
		expect(stepBack(emptyTimeline())).toBeUndefined();
		expect(stepForward(build(["a"]))).toBeUndefined();
	});

	test("a push after undo discards the redo tail (defect: replaying a divergent branch)", () => {
		const timeline = must(
			stepBack(must(stepBack(build(["a"], ["b"], ["c"]))).timeline),
		).timeline;
		const pushed = pushEntry(timeline, [command("d")], 1);
		expect(seqs(pushed.timeline)).toEqual([1, 4]);
		expect(pushed.removed.map((entry) => entry.seq)).toEqual([2, 3]);
		expect(pushed.timeline.cursor).toBe(2);
		expect(summarizeTimeline(pushed.timeline)).toEqual({
			canUndo: true,
			canRedo: false,
			undoDepth: 2,
			redoDepth: 0,
		});
	});

	test("sequence numbers never repeat, even after the tail was discarded", () => {
		const timeline = must(stepBack(build(["a"], ["b"]))).timeline;
		const first = pushEntry(timeline, [command("c")], 1);
		const second = pushEntry(first.timeline, [command("d")], 1);
		expect(first.entry.seq).toBe(3);
		expect(second.entry.seq).toBe(4);
	});

	test("entry cap evicts the oldest applied entries", () => {
		let timeline = emptyTimeline();
		const removed: HistoryEntry[] = [];
		for (let i = 0; i < 5; i++) {
			const pushed = pushEntry(timeline, [command(`n${i}`)], 1, {
				maxEntries: 3,
				maxBytes: Number.POSITIVE_INFINITY,
			});
			timeline = pushed.timeline;
			removed.push(...pushed.removed);
		}
		expect(seqs(timeline)).toEqual([3, 4, 5]);
		expect(removed.map((entry) => entry.seq)).toEqual([1, 2]);
		expect(timeline.cursor).toBe(3);
	});

	test("byte cap evicts oldest first but always keeps the newest entry", () => {
		const big = [command("x".repeat(500))];
		const first = pushEntry(emptyTimeline(), big, 1, {
			maxEntries: 100,
			maxBytes: 100,
		});
		expect(seqs(first.timeline)).toEqual([1]);
		const second = pushEntry(first.timeline, big, 1, {
			maxEntries: 100,
			maxBytes: 100,
		});
		expect(seqs(second.timeline)).toEqual([2]);
		expect(second.removed.map((entry) => entry.seq)).toEqual([1]);
	});
});

describe("failure handling", () => {
	test("withCursor clamps into range", () => {
		const timeline = build(["a"], ["b"]);
		expect(withCursor(timeline, -3).cursor).toBe(0);
		expect(withCursor(timeline, 9).cursor).toBe(2);
		expect(withCursor(timeline, 1).cursor).toBe(1);
	});

	test("discarding an applied entry keeps the cursor on the same neighbours", () => {
		const timeline = build(["a"], ["b"], ["c"]);
		const undone = must(stepBack(timeline));
		const discarded = discardEntry(undone.timeline, undone.entry);
		expect(seqs(discarded)).toEqual([1, 2]);
		expect(discarded.cursor).toBe(2);
		expect(appliedSeq(discarded)).toBe(2);
	});

	test("discarding a redo entry shrinks the redo tail", () => {
		const timeline = must(stepBack(build(["a"], ["b"]))).timeline;
		const redone = must(stepForward(timeline));
		const discarded = discardEntry(redone.timeline, redone.entry);
		expect(seqs(discarded)).toEqual([1]);
		expect(discarded.cursor).toBe(1);
	});

	test("discarding an unknown entry is a no-op", () => {
		const timeline = build(["a"]);
		expect(
			discardEntry(timeline, { seq: 99, at: 0, payload: "[]", bytes: 2 }),
		).toBe(timeline);
	});

	test("decodeEntry rejects payloads that are not a batch", () => {
		expect(() =>
			decodeEntry({ seq: 1, at: 0, payload: '{"a":1}', bytes: 7 }),
		).toThrow(/does not hold a command batch/);
		expect(() =>
			decodeEntry({ seq: 1, at: 0, payload: "not json", bytes: 8 }),
		).toThrow();
	});
});

describe("hydration", () => {
	const row = (seq: number): HistoryEntry => ({
		seq,
		at: seq,
		payload: JSON.stringify([command(`n${seq}`)]),
		bytes: 10,
	});

	test("rebuilds the cursor from the applied sequence and sorts rows", () => {
		const timeline = timelineFromRows([row(3), row(1), row(2)], 2);
		expect(seqs(timeline)).toEqual([1, 2, 3]);
		expect(timeline.cursor).toBe(2);
		expect(timeline.nextSeq).toBe(4);
	});

	test("an applied sequence past the last row applies everything", () => {
		const timeline = timelineFromRows([row(1), row(2)], 50, 60);
		expect(timeline.cursor).toBe(2);
		expect(timeline.nextSeq).toBe(60);
	});

	test("an applied sequence of zero leaves everything redoable", () => {
		const timeline = timelineFromRows([row(1), row(2)], 0);
		expect(timeline.cursor).toBe(0);
		expect(summarizeTimeline(timeline).redoDepth).toBe(2);
	});

	test("empty rows produce a fresh timeline with a sequence floor of one", () => {
		expect(timelineFromRows([], 0)).toEqual(emptyTimeline());
	});
});

describe("remote board applied events", () => {
	test("reads app, board and reason; unknown reasons count as sync", () => {
		const event = new CustomEvent("flow:remote-board-applied", {
			detail: { appId: "a", boardId: "b", reason: "reset" },
		});
		expect(readRemoteBoardApplied(event)).toEqual({
			appId: "a",
			boardId: "b",
			reason: "reset",
		});
		const legacy = new CustomEvent("flow:remote-board-applied", {
			detail: { appId: "a", boardId: "b" },
		});
		expect(readRemoteBoardApplied(legacy)?.reason).toBe("sync");
	});

	test("preserves the exact boards involved in a remote merge", () => {
		const before = { id: "b", nodes: {} } as IBoard;
		const after = { ...before, name: "Updated flow" };
		const merge = { before, after };
		const event = new CustomEvent("flow:remote-board-applied", {
			detail: { appId: "a", boardId: "b", reason: "sync", merge },
		});
		const detail = readRemoteBoardApplied(event);
		expect(detail?.merge).toBe(merge);
		expect(detail?.merge?.before).toBe(before);
		expect(detail?.merge?.after).toBe(after);
	});

	test("ignores an incomplete merge snapshot without dropping the event", () => {
		const event = new CustomEvent("flow:remote-board-applied", {
			detail: {
				appId: "a",
				boardId: "b",
				merge: { before: { id: "b", nodes: {} } },
			},
		});
		expect(readRemoteBoardApplied(event)).toEqual({
			appId: "a",
			boardId: "b",
			reason: "sync",
		});
	});

	test("ignores malformed events", () => {
		expect(readRemoteBoardApplied(new Event("x"))).toBeUndefined();
		expect(
			readRemoteBoardApplied(
				new CustomEvent("x", { detail: { appId: "only" } }),
			),
		).toBeUndefined();
	});
});

describe("isTransientReplayError", () => {
	test("transport, lock and availability failures are transient", () => {
		expect(isTransientReplayError(new TypeError("Failed to fetch"))).toBe(true);
		expect(isTransientReplayError(new Error("Network unavailable: x"))).toBe(
			true,
		);
		expect(isTransientReplayError({ status: 423, message: "locked" })).toBe(
			true,
		);
		expect(isTransientReplayError({ status: 503, message: "down" })).toBe(true);
		expect(isTransientReplayError({ status: 401, message: "auth" })).toBe(true);
		expect(
			isTransientReplayError(
				"Board edits are paused while FlowPilot finishes durable delivery",
			),
		).toBe(true);
	});

	test("a refusal by the board is not transient", () => {
		expect(isTransientReplayError(new Error("Node abc not found"))).toBe(false);
		expect(
			isTransientReplayError({ status: 422, message: "validation failed" }),
		).toBe(false);
		expect(isTransientReplayError({ status: 400, message: "bad" })).toBe(false);
	});
});

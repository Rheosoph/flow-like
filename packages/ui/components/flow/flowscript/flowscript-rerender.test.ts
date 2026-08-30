import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parseFlowScriptAnchors } from "./flowscript-anchors";
import {
	FLOWSCRIPT_TYPING_QUIESCENCE_MS,
	captureFlowScriptSeat,
	createDeferredReloadRunner,
	resolveFlowScriptSeat,
	resolveFlowScriptSeatPoint,
	shouldDeferFlowScriptReload,
} from "./flowscript-rerender";

const FIXTURE_DIR = join(import.meta.dir, "../../../../../tests/ast");

const X_ID = "nodex000000000000001";
const Y_ID = "nodey000000000000001";
const LOG_ID = "nodelog0000000000001";

const OLD_TEXT = [
	"use std::*",
	"",
	"eventsGeneric main(payload: Struct) {",
	`    const x = now()   //@n:${X_ID}`,
	`    const y = add({ a: x })   //@n:${Y_ID}`,
	"}",
	"",
	"eventsGeneric other(payload: Struct) {",
	`    log({ msg: "hi" })   //@n:${LOG_ID}`,
	"}",
].join("\n");

/** Same board re-rendered with a longer use block: every anchor moved down 3 lines. */
const NEW_TEXT = [
	"use std::*",
	"use http::*",
	"use json::*",
	"use db::*",
	"",
	"eventsGeneric main(payload: Struct) {",
	`    const x = now()   //@n:${X_ID}`,
	`    const y = add({ a: x })   //@n:${Y_ID}`,
	"}",
	"",
	"eventsGeneric other(payload: Struct) {",
	`    log({ msg: "hi" })   //@n:${LOG_ID}`,
	"}",
].join("\n");

const oldIndex = parseFlowScriptAnchors(OLD_TEXT);
const newIndex = parseFlowScriptAnchors(NEW_TEXT);
const newMaxLine = NEW_TEXT.split("\n").length;

describe("seat capture and reseat across a changed render", () => {
	test("a cursor on an anchored statement follows its anchor", () => {
		const seat = captureFlowScriptSeat(oldIndex, {
			position: { lineNumber: 5, column: 17 },
		});
		expect(seat.cursor.anchorId).toBe(Y_ID);
		expect(seat.cursor.dLine).toBe(0);
		const resolved = resolveFlowScriptSeat(
			seat,
			oldIndex,
			newIndex,
			newMaxLine,
		);
		expect(resolved.position).toEqual({ lineNumber: 8, column: 17 });
	});

	test("a cursor on an unanchored line keeps its offset below the owning anchor", () => {
		// Line 6 is the closing brace under the y statement (anchor at line 5).
		const seat = captureFlowScriptSeat(oldIndex, {
			position: { lineNumber: 6, column: 1 },
		});
		expect(seat.cursor).toMatchObject({ anchorId: Y_ID, dLine: 1 });
		const resolved = resolveFlowScriptSeat(
			seat,
			oldIndex,
			newIndex,
			newMaxLine,
		);
		expect(resolved.position.lineNumber).toBe(9);
	});

	test("selection and scroll resolve alongside the cursor", () => {
		const seat = captureFlowScriptSeat(oldIndex, {
			position: { lineNumber: 5, column: 10 },
			selectionStart: { lineNumber: 4, column: 5 },
			firstVisibleLine: 4,
			firstVisibleLineOffsetPx: 7,
		});
		const resolved = resolveFlowScriptSeat(
			seat,
			oldIndex,
			newIndex,
			newMaxLine,
		);
		expect(resolved.position).toEqual({ lineNumber: 8, column: 10 });
		expect(resolved.selectionStart).toEqual({ lineNumber: 7, column: 5 });
		expect(resolved.scroll).toEqual({ lineNumber: 7, offsetPx: 7 });
	});

	test("a vanished anchor falls back to the nearest surviving anchor", () => {
		const withoutY = parseFlowScriptAnchors(
			NEW_TEXT.split("\n")
				.filter((line) => !line.includes(`//@n:${Y_ID}`))
				.join("\n"),
		);
		const resolved = resolveFlowScriptSeatPoint(
			{ anchorId: Y_ID, dLine: 0, line: 5, column: 3 },
			oldIndex,
			withoutY,
			12,
		);
		// Nearest old anchor to line 5 that survives is X (old line 4): its new
		// line + the point's offset to it (5 - 4 = 1).
		const xNewLine = withoutY.firstLineById.get(X_ID);
		expect(xNewLine).toBeDefined();
		expect(resolved.lineNumber).toBe((xNewLine ?? 0) + 1);
	});

	test("with no surviving anchors the same line number wins, clamped to the document", () => {
		const emptyIndex = parseFlowScriptAnchors("just one line");
		const resolved = resolveFlowScriptSeatPoint(
			{ anchorId: X_ID, dLine: 0, line: 40, column: 2 },
			oldIndex,
			emptyIndex,
			1,
		);
		expect(resolved).toEqual({ lineNumber: 1, column: 2 });
	});

	test("fixture-backed: a seat deep in a real render survives statement insertion above", () => {
		const baseline = readFileSync(
			join(FIXTURE_DIR, "ttwctnp08u18sg2z6nmcqqak.anchored.flow"),
			"utf8",
		);
		const index = parseFlowScriptAnchors(baseline);
		const anchorLine = index.firstLineById.get("dcc9b9ioxr85bjr1t6kt0cyt");
		if (!anchorLine) throw new Error("fixture anchor missing");
		const seat = captureFlowScriptSeat(index, {
			position: { lineNumber: anchorLine, column: 12 },
		});
		const lines = baseline.split("\n");
		lines.splice(anchorLine - 5, 0, "    inserted()", "    inserted2()");
		const shifted = lines.join("\n");
		const shiftedIndex = parseFlowScriptAnchors(shifted);
		const resolved = resolveFlowScriptSeat(
			seat,
			index,
			shiftedIndex,
			lines.length,
		);
		expect(resolved.position.lineNumber).toBe(anchorLine + 2);
		expect(lines[resolved.position.lineNumber - 1]).toContain(
			"//@n:dcc9b9ioxr85bjr1t6kt0cyt",
		);
	});
});

describe("reload deferral guard", () => {
	const base = {
		now: 10_000,
		editorFocused: false,
		lastInputAt: undefined as number | undefined,
		composing: false,
		widgetOpen: false,
	};

	test("an idle, unfocused editor never defers", () => {
		expect(shouldDeferFlowScriptReload(base)).toBe(false);
	});

	test("IME composition always defers", () => {
		expect(shouldDeferFlowScriptReload({ ...base, composing: true })).toBe(
			true,
		);
	});

	test("an open widget always defers, focus or not", () => {
		expect(shouldDeferFlowScriptReload({ ...base, widgetOpen: true })).toBe(
			true,
		);
	});

	test("recent typing defers only while the editor is focused", () => {
		const typing = { ...base, lastInputAt: 9_500 };
		expect(shouldDeferFlowScriptReload(typing)).toBe(false);
		expect(
			shouldDeferFlowScriptReload({ ...typing, editorFocused: true }),
		).toBe(true);
	});

	test("typing quiescence clears exactly at the window boundary", () => {
		const focused = { ...base, editorFocused: true };
		expect(
			shouldDeferFlowScriptReload({
				...focused,
				lastInputAt: base.now - FLOWSCRIPT_TYPING_QUIESCENCE_MS + 1,
			}),
		).toBe(true);
		expect(
			shouldDeferFlowScriptReload({
				...focused,
				lastInputAt: base.now - FLOWSCRIPT_TYPING_QUIESCENCE_MS,
			}),
		).toBe(false);
	});
});

function manualScheduler() {
	let queue: { cb: () => void; id: number }[] = [];
	let nextId = 1;
	return {
		schedule: (cb: () => void, _ms: number) => {
			const id = nextId++;
			queue.push({ cb, id });
			return id as unknown;
		},
		cancel: (handle: unknown) => {
			queue = queue.filter((entry) => entry.id !== handle);
		},
		fire: () => {
			const pending = queue.splice(0);
			for (const entry of pending) entry.cb();
		},
		size: () => queue.length,
	};
}

describe("deferred reload runner", () => {
	test("runs immediately when unblocked", () => {
		const scheduler = manualScheduler();
		let runs = 0;
		const runner = createDeferredReloadRunner({
			run: () => runs++,
			isBlocked: () => false,
			schedule: scheduler.schedule,
			cancel: scheduler.cancel,
		});
		runner.request();
		expect(runs).toBe(1);
		expect(runner.pending()).toBe(false);
		expect(scheduler.size()).toBe(0);
	});

	test("a blocked request defers (never drops) and coalesces repeats", () => {
		const scheduler = manualScheduler();
		let blocked = true;
		let runs = 0;
		const runner = createDeferredReloadRunner({
			run: () => runs++,
			isBlocked: () => blocked,
			schedule: scheduler.schedule,
			cancel: scheduler.cancel,
		});
		runner.request();
		runner.request();
		runner.request();
		expect(runs).toBe(0);
		expect(runner.pending()).toBe(true);
		// Still blocked on the first check: re-arms instead of running.
		scheduler.fire();
		expect(runs).toBe(0);
		expect(scheduler.size()).toBe(1);
		blocked = false;
		scheduler.fire();
		expect(runs).toBe(1);
		expect(runner.pending()).toBe(false);
	});

	test("poke flushes a pending run once unblocked (blur path)", () => {
		const scheduler = manualScheduler();
		let blocked = true;
		let runs = 0;
		const runner = createDeferredReloadRunner({
			run: () => runs++,
			isBlocked: () => blocked,
			schedule: scheduler.schedule,
			cancel: scheduler.cancel,
		});
		runner.request();
		runner.poke();
		expect(runs).toBe(0);
		blocked = false;
		runner.poke();
		expect(runs).toBe(1);
		// The armed timer was cancelled with the flush.
		scheduler.fire();
		expect(runs).toBe(1);
	});

	test("poke without a pending request does nothing", () => {
		const scheduler = manualScheduler();
		let runs = 0;
		const runner = createDeferredReloadRunner({
			run: () => runs++,
			isBlocked: () => false,
			schedule: scheduler.schedule,
			cancel: scheduler.cancel,
		});
		runner.poke();
		expect(runs).toBe(0);
	});

	test("dispose cancels the pending run", () => {
		const scheduler = manualScheduler();
		let runs = 0;
		const runner = createDeferredReloadRunner({
			run: () => runs++,
			isBlocked: () => true,
			schedule: scheduler.schedule,
			cancel: scheduler.cancel,
		});
		runner.request();
		runner.dispose();
		scheduler.fire();
		expect(runs).toBe(0);
		runner.request();
		expect(runs).toBe(0);
	});
});

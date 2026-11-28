import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import {
	holdChatScrollMark,
	readChatScrollMark,
	recallChatScroll,
	rememberChatScroll,
} from "./chat-scroll-memory";

const VIEWPORT_TOP = 50;
const VIEWPORT_HEIGHT = 400;

interface FakeRow {
	id: string;
	height: number;
	connected: boolean;
}

/** A scroll container laid out as a plain column of rows, measured like the browser would. */
function transcript(heights: number[]) {
	const rows: FakeRow[] = heights.map((height, index) => ({
		id: `m${index}`,
		height,
		connected: true,
	}));
	const listeners = new Map<string, Set<() => void>>();
	const container = {
		scrollTop: 0,
		clientHeight: VIEWPORT_HEIGHT,
		getBoundingClientRect: () => ({ top: VIEWPORT_TOP }),
		addEventListener: (type: string, listener: () => void) => {
			const set = listeners.get(type) ?? new Set();
			set.add(listener);
			listeners.set(type, set);
		},
		removeEventListener: (type: string, listener: () => void) => {
			listeners.get(type)?.delete(listener);
		},
		get children() {
			return rows.map((row) => ({
				hasAttribute: (name: string) => name === "data-chat-message",
				getAttribute: () => row.id,
				get isConnected() {
					return row.connected;
				},
				getBoundingClientRect: () => {
					const index = rows.indexOf(row);
					const top =
						VIEWPORT_TOP +
						rows.slice(0, index).reduce((sum, r) => sum + r.height, 0) -
						container.scrollTop;
					return { top, bottom: top + row.height };
				},
			}));
		},
	};
	const fire = (type: string) => {
		for (const listener of [...(listeners.get(type) ?? [])]) listener();
	};
	const listenerCount = () =>
		[...listeners.values()].reduce((sum, set) => sum + set.size, 0);
	return {
		rows,
		container: container as unknown as HTMLElement,
		raw: container,
		fire,
		listenerCount,
	};
}

let frames: Array<() => void> = [];
const globals = globalThis as {
	requestAnimationFrame?: unknown;
	cancelAnimationFrame?: unknown;
};
const original = {
	request: globals.requestAnimationFrame,
	cancel: globals.cancelAnimationFrame,
};
const runFrame = () => {
	const queued = frames;
	frames = [];
	for (const frame of queued) frame();
};

beforeEach(() => {
	frames = [];
	globals.requestAnimationFrame = (callback: () => void) => {
		frames.push(callback);
		return frames.length;
	};
	globals.cancelAnimationFrame = () => {
		frames = [];
	};
});

afterEach(() => {
	globals.requestAnimationFrame = original.request;
	globals.cancelAnimationFrame = original.cancel;
});

describe("chat scroll memory", () => {
	test("remembers per session and forgets a chat left at the end", () => {
		rememberChatScroll("a", { rowId: "m3", offset: -20 });
		rememberChatScroll("b", { rowId: "m7", offset: 0 });
		expect(recallChatScroll("a")).toEqual({ rowId: "m3", offset: -20 });
		expect(recallChatScroll("b")).toEqual({ rowId: "m7", offset: 0 });
		rememberChatScroll("a", null);
		expect(recallChatScroll("a")).toBeUndefined();
		expect(recallChatScroll(undefined)).toBeUndefined();
	});

	test("keeps only the most recently left sessions", () => {
		for (let index = 0; index < 70; index++) {
			rememberChatScroll(`s${index}`, { rowId: "m0", offset: 0 });
		}
		expect(recallChatScroll("s0")).toBeUndefined();
		expect(recallChatScroll("s69")).toEqual({ rowId: "m0", offset: 0 });
	});

	test("marks the first row reaching into view and its offset", () => {
		const { container, raw } = transcript([100, 100, 300, 100, 100]);
		raw.scrollTop = 250;
		expect(readChatScrollMark(container)).toEqual({
			rowId: "m2",
			offset: -50,
		});
		raw.scrollTop = 200;
		expect(readChatScrollMark(container)).toEqual({ rowId: "m2", offset: 0 });
	});

	test("reads nothing from a hidden container", () => {
		const { container, raw } = transcript([100, 100]);
		raw.clientHeight = 0;
		expect(readChatScrollMark(container)).toBeNull();
	});

	test("restores the row and holds it while rows above it grow", () => {
		const { container, raw, rows } = transcript([96, 96, 96, 300, 200, 200]);
		const release = holdChatScrollMark(container, {
			rowId: "m3",
			offset: -40,
		});
		expect(release).not.toBeNull();
		expect(raw.scrollTop).toBe(96 * 3 + 40);

		rows[1].height = 250;
		runFrame();
		expect(raw.scrollTop).toBe(96 * 2 + 250 + 40);
		expect(readChatScrollMark(container)).toEqual({
			rowId: "m3",
			offset: -40,
		});
		release?.();
	});

	test("hands the scroll back on the reader's first input", () => {
		const { container, raw, rows, fire, listenerCount } = transcript([
			96, 96, 300, 200,
		]);
		holdChatScrollMark(container, { rowId: "m2", offset: 0 });
		expect(listenerCount()).toBeGreaterThan(0);
		fire("wheel");
		expect(listenerCount()).toBe(0);

		const placed = raw.scrollTop;
		rows[0].height = 400;
		runFrame();
		expect(raw.scrollTop).toBe(placed);
	});

	test("does nothing when the row is not in the transcript", () => {
		const { container, raw, listenerCount } = transcript([100, 100]);
		expect(holdChatScrollMark(container, { rowId: "gone", offset: 0 })).toBe(
			null,
		);
		expect(raw.scrollTop).toBe(0);
		expect(listenerCount()).toBe(0);
	});
});

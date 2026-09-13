import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import Graph from "graphology";
import { loadGraphScene, saveGraphScene } from "./graph-position-store";

const STORAGE_KEY = "flow-like:graph-scene:test-layout";
let originalWindow: PropertyDescriptor | undefined;
let storage: Map<string, string>;

beforeEach(() => {
	originalWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
	storage = new Map();
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: {
			localStorage: {
				getItem: (key: string) => storage.get(key) ?? null,
				setItem: (key: string, value: string) => storage.set(key, value),
			},
		},
	});
});

afterEach(() => {
	if (originalWindow) {
		Object.defineProperty(globalThis, "window", originalWindow);
	} else {
		Reflect.deleteProperty(globalThis, "window");
	}
});

describe("saved graph layouts", () => {
	test("rebuilds legacy automatic positions while retaining valid manual pins", () => {
		storage.set(
			STORAGE_KEY,
			JSON.stringify({
				v: 1,
				positions: {
					feed: [-1200, 1],
					article: [1400, 2],
					pinned: [100, 200],
					invalid: [null, 3],
				},
				pinned: ["pinned", "missing", "invalid", 7],
			}),
		);
		const scene = loadGraphScene("test-layout");
		expect(scene?.needsLayout).toBe(true);
		expect(scene?.positions).toEqual(new Map([["pinned", { x: 100, y: 200 }]]));
		expect(scene?.pinned).toEqual(new Set(["pinned"]));
	});

	test("requests a new layout when a legacy scene has no pinned nodes", () => {
		storage.set(
			STORAGE_KEY,
			JSON.stringify({ v: 1, positions: { feed: [1000, 0] }, pinned: [] }),
		);
		const scene = loadGraphScene("test-layout");
		expect(scene?.needsLayout).toBe(true);
		expect(scene?.positions.size).toBe(0);
	});

	test("restores all positions after saving with the current layout version", () => {
		const graph = new Graph();
		graph.addNode("feed", { x: -100.25, y: 200.75 });
		graph.addNode("article", { x: 120, y: -75 });
		saveGraphScene("test-layout", graph, new Set(["feed"]));
		const scene = loadGraphScene("test-layout");
		expect(scene?.needsLayout).toBe(false);
		expect(scene?.positions.get("feed")).toEqual({ x: -100.2, y: 200.8 });
		expect(scene?.positions.get("article")).toEqual({ x: 120, y: -75 });
		expect(scene?.pinned).toEqual(new Set(["feed"]));
	});

	test("ignores malformed or unknown saved formats", () => {
		for (const raw of [
			"{broken",
			JSON.stringify({ v: 1, positions: null }),
			JSON.stringify({ v: 99, positions: {}, pinned: [] }),
		]) {
			storage.set(STORAGE_KEY, raw);
			expect(loadGraphScene("test-layout")).toBeNull();
		}
	});
});

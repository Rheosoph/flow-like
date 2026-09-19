import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import {
	EMPTY_STRING_REF,
	collectBoardRefs,
	resolveBoardRef,
} from "./board-refs";
import type { IBoard } from "./schema/flow/board";

const window = new Window();
Object.assign(window, { SyntaxError, TypeError });
Object.assign(globalThis, {
	document: window.document,
	Element: window.Element,
	HTMLElement: window.HTMLElement,
	HTMLInputElement: window.HTMLInputElement,
	HTMLSelectElement: window.HTMLSelectElement,
	HTMLTextAreaElement: window.HTMLTextAreaElement,
	navigator: window.navigator,
	window,
});

const { handleCopy, handlePaste } = await import("./flow-board-utils");

const REFS = {
	node_key: "Fires when an order arrives",
	pin_key: "Begins order handling",
	layer_pin_key: "The customer id",
	schema_key: '{"type":"object"}',
	unrelated_key: "Not copied",
};

function pin(id: string, description: string, schema?: string) {
	return {
		id,
		name: id,
		friendly_name: id,
		description,
		pin_type: "Output",
		data_type: "String",
		value_type: "Normal",
		depends_on: [],
		connected_to: [],
		index: 1,
		schema,
	};
}

const board = {
	nodes: {
		event: {
			id: "event",
			name: "events_generic",
			friendly_name: "Order received",
			description: "node_key",
			category: "Events",
			pins: { exec_out: pin("exec_out", "pin_key") },
		},
	},
	layers: {
		layer: {
			id: "layer",
			name: "Layer",
			pins: { input: pin("input", "layer_pin_key", "schema_key") },
		},
	},
	comments: {},
	variables: {},
	refs: REFS,
} as unknown as IBoard;

describe("resolveBoardRef", () => {
	test("resolves a key, passes plain catalog text through, and blanks the empty-string key", () => {
		expect(resolveBoardRef("node_key", REFS)).toBe(REFS.node_key);
		expect(resolveBoardRef("A generic event", REFS)).toBe("A generic event");
		expect(resolveBoardRef(EMPTY_STRING_REF, REFS)).toBe("");
		expect(resolveBoardRef(undefined, REFS)).toBe("");
	});
});

describe("collectBoardRefs", () => {
	test("copies only the refs that exist", () => {
		expect(collectBoardRefs(["node_key", "plain text", null], REFS)).toEqual({
			node_key: REFS.node_key,
		});
	});
});

describe("board clipboard", () => {
	test("a copied node and layer paste with their descriptions and the refs behind them", async () => {
		let clipboard = "";
		Object.defineProperty(globalThis.navigator, "clipboard", {
			configurable: true,
			value: {
				writeText: async (text: string) => {
					clipboard = text;
				},
				readText: async () => clipboard,
			},
		});

		handleCopy(
			[
				{ id: "event", selected: true, type: "node" },
				{
					id: "layer",
					selected: true,
					type: "layerNode",
					data: { layer: board.layers.layer },
				},
			],
			board,
			{ x: 0, y: 0 },
		);

		let pasted: Record<string, any> | undefined;
		await handlePaste(
			{
				preventDefault: () => {},
				stopPropagation: () => {},
				target: null,
			} as unknown as ClipboardEvent,
			{ x: 10, y: 10 },
			"board",
			async (command) => {
				pasted = command;
			},
		);

		expect(pasted).toBeDefined();
		const [node] = pasted?.original_nodes ?? [];
		expect(node.description).toBe("node_key");
		expect(node.pins.exec_out.description).toBe("pin_key");
		expect(pasted?.original_refs).toEqual({
			node_key: REFS.node_key,
			pin_key: REFS.pin_key,
			layer_pin_key: REFS.layer_pin_key,
			schema_key: REFS.schema_key,
		});
	});
});

describe("hostile ref keys", () => {
	test("inherited Object members are never treated as refs", () => {
		const refs = { node_key: "text" } as IBoard["refs"];
		expect(resolveBoardRef("constructor", refs)).toBe("constructor");
		expect(resolveBoardRef("__proto__", refs)).toBe("__proto__");
		expect(resolveBoardRef("hasOwnProperty", refs)).toBe("hasOwnProperty");
		expect(
			collectBoardRefs(["constructor", "__proto__", "node_key"], refs),
		).toEqual({
			node_key: "text",
		});
	});
});

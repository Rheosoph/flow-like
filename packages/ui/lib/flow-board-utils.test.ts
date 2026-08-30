import { describe, expect, test } from "bun:test";
import { doPinsMatch, parseBoard } from "./flow-board-utils";
import type {
	IBoard,
	ILayer,
	ILayerCache,
	IVariable,
} from "./schema/flow/board";
import { ILayerCacheScope, ILayerType } from "./schema/flow/board";
import type { INode } from "./schema/flow/node";
import type { IPin } from "./schema/flow/pin";
import { IVariableType } from "./schema/flow/node";
import { IPinType, IValueType } from "./schema/flow/pin";
import { convertJsonToUint8Array, parseUint8ArrayToJson } from "./uint8";

/** Only the slice of a rendered React Flow node these tests read. */
interface IRenderedNode {
	id: string;
	data: {
		node: INode;
		boardDataVersion?: string;
		boardContentVersion?: string;
		functionCache?: ILayerCache;
	};
}

const variable = (id: string, name: string): IVariable =>
	({
		id,
		name,
		category: null,
		data_type: IVariableType.String,
		value_type: IValueType.Normal,
		exposed: false,
		secret: false,
		editable: true,
	}) as IVariable;

const getNode = (id: string, varRef: string): INode =>
	({
		id,
		name: "variable_get",
		friendly_name: `Get ${varRef}`,
		description: "",
		category: "Variables",
		coordinates: [0, 0, 0],
		hash: 1,
		pins: {
			p1: {
				id: "p1",
				name: "var_ref",
				friendly_name: "Variable",
				description: "",
				pin_type: "Input",
				data_type: IVariableType.String,
				value_type: IValueType.Normal,
				index: 0,
				connected_to: [],
				depends_on: [],
				default_value: convertJsonToUint8Array(varRef),
			},
		},
	}) as unknown as INode;

const functionLayer = (id: string, overrides: Partial<ILayer> = {}): ILayer =>
	({
		id,
		name: "fn",
		type: ILayerType.Function,
		nodes: {},
		pins: {},
		variables: {},
		comments: {},
		coordinates: [0, 0, 0],
		parent_id: null,
		...overrides,
	}) as unknown as ILayer;

const board = (overrides: Partial<IBoard> = {}): IBoard =>
	({
		id: "board-1",
		name: "b",
		nodes: { n1: getNode("n1", "v1") },
		variables: { v1: variable("v1", "Counter") },
		layers: { l1: functionLayer("l1") },
		comments: {},
		refs: {},
		version: [0, 0, 1],
		...overrides,
	}) as unknown as IBoard;

interface IParsed {
	nodes: IRenderedNode[];
	edges: unknown[];
}

const parse = (
	source: IBoard,
	oldNodes?: IRenderedNode[],
	oldEdges?: unknown[],
): IParsed =>
	parseBoard(
		source,
		"app-1",
		() => {},
		() => {},
		async () => {},
		async () => undefined,
		new Set<string>(),
		undefined,
		oldNodes,
		oldEdges,
		undefined,
		{ current: source },
	) as IParsed;

const node = (parsed: IParsed, id: string): IRenderedNode => {
	const found = parsed.nodes.find((candidate) => candidate.id === id);
	if (!found) throw new Error(`node ${id} was not rendered`);
	return found;
};

const contentVersion = (parsed: IParsed, id: string) =>
	node(parsed, id).data.boardContentVersion;

const callNode = (id: string, layerId: string): INode =>
	({
		id,
		name: "control_call_function",
		friendly_name: "Call fn",
		description: "",
		category: "Control",
		coordinates: [0, 0, 0],
		hash: 2,
		pins: {
			p1: {
				id: "p1",
				name: "function_layer_id",
				friendly_name: "Function",
				description: "",
				pin_type: "Input",
				data_type: IVariableType.String,
				value_type: IValueType.Normal,
				index: 0,
				connected_to: [],
				depends_on: [],
				default_value: convertJsonToUint8Array(layerId),
			},
		},
	}) as unknown as INode;

/**
 * The version tokens are identity-based, relying on react-query's structural
 * sharing to keep unchanged sub-objects referentially stable across refetches.
 * Spreading the previous board reproduces that: every slice keeps its reference
 * except the one the test actually changes.
 */
const edit = (previous: IBoard, change: Partial<IBoard>): IBoard => ({
	...previous,
	...change,
});

describe("boardContentVersion on rendered nodes", () => {
	test("changes when a variable is renamed, so variable nodes re-render", () => {
		const before = board();
		const first = parse(before);
		const token = contentVersion(first, "n1");
		expect(token).toBeString();

		// A rename replaces the variables map. The node itself is untouched, so its
		// backend hash — the memo comparator's primary key — does not move, and the
		// content version is the only signal that its pins must re-resolve.
		const second = parse(
			edit(before, { variables: { v1: variable("v1", "Renamed") } }),
			first.nodes,
			first.edges,
		);

		expect(node(second, "n1").data.node.hash).toBe(
			node(first, "n1").data.node.hash,
		);
		expect(contentVersion(second, "n1")).not.toBe(token);
	});

	test("changes when a function layer is renamed", () => {
		const before = board();
		const first = parse(before);
		const second = parse(
			edit(before, {
				layers: { l1: functionLayer("l1", { name: "renamed" }) },
			}),
			first.nodes,
			first.edges,
		);
		expect(contentVersion(second, "n1")).not.toBe(contentVersion(first, "n1"));
	});

	test("changes when a LOCAL variable is renamed", () => {
		// Local variables live on the function layer, and a var_ref pin lists them
		// alongside the board's own — a rename there must reach the node too.
		const before = board({
			layers: {
				l1: functionLayer("l1", { variables: { lv: variable("lv", "Local") } }),
			},
		});
		const first = parse(before);
		const second = parse(
			edit(before, {
				layers: {
					l1: functionLayer("l1", {
						variables: { lv: variable("lv", "LocalRenamed") },
					}),
				},
			}),
			first.nodes,
			first.edges,
		);
		expect(contentVersion(second, "n1")).not.toBe(contentVersion(first, "n1"));
	});

	test("changes when a local variable MOVES to another layer", () => {
		// The dropdown a var_ref pin renders lists the current layer's locals, so a
		// variable changing owner changes what a mounted node offers.
		const before = board({
			layers: {
				l1: functionLayer("l1", { variables: { lv: variable("lv", "Local") } }),
				l2: functionLayer("l2", { name: "other" }),
			},
		});
		const first = parse(before);
		const second = parse(
			edit(before, {
				layers: {
					l1: functionLayer("l1"),
					l2: functionLayer("l2", {
						name: "other",
						variables: { lv: variable("lv", "Local") },
					}),
				},
			}),
			first.nodes,
			first.edges,
		);
		expect(contentVersion(second, "n1")).not.toBe(contentVersion(first, "n1"));
	});

	test("changes when a local variable is RETYPED", () => {
		const before = board({
			layers: {
				l1: functionLayer("l1", { variables: { lv: variable("lv", "Local") } }),
			},
		});
		const first = parse(before);
		const second = parse(
			edit(before, {
				layers: {
					l1: functionLayer("l1", {
						variables: {
							lv: {
								...variable("lv", "Local"),
								data_type: IVariableType.Integer,
							},
						},
					}),
				},
			}),
			first.nodes,
			first.edges,
		);
		expect(contentVersion(second, "n1")).not.toBe(contentVersion(first, "n1"));
	});

	test("does not change when a layer is DRAGGED", () => {
		// The whole point of the signature: coordinates move constantly and no node
		// renders them, so a layer drag must not repaint the canvas.
		const before = board();
		const first = parse(before);
		const token = contentVersion(first, "n1");

		const second = parse(
			edit(before, {
				layers: { l1: functionLayer("l1", { coordinates: [640, 480, 0] }) },
			}),
			first.nodes,
			first.edges,
		);

		expect(node(second, "n1").data.boardDataVersion).not.toBe(
			node(first, "n1").data.boardDataVersion,
		);
		expect(contentVersion(second, "n1")).toBe(token);
	});

	test("refreshes functionCache when the called function toggles caching", () => {
		// A call node's own hash does not move when the function it points at
		// changes its caching, so every rebuild path has to recompute the badge.
		const before = board({
			nodes: { c1: callNode("c1", "l1") },
			layers: { l1: functionLayer("l1") },
		});
		const first = parse(before);
		expect(node(first, "c1").data.functionCache).toBeUndefined();

		const second = parse(
			edit(before, {
				layers: {
					l1: functionLayer("l1", {
						cache: {
							enabled: true,
							prefix: "p",
							ttl_seconds: 60,
							scope: ILayerCacheScope.App,
						},
					}),
				},
			}),
			first.nodes,
			first.edges,
		);

		// Same node hash — this is the shallow-update path, not a full rebuild.
		expect(node(second, "c1").data.node.hash).toBe(
			node(first, "c1").data.node.hash,
		);
		expect(node(second, "c1").data.functionCache?.enabled).toBe(true);
		expect(node(second, "c1").data.functionCache?.ttl_seconds).toBe(60);
	});

	test("keeps the function_layer_id pin on the rendered call node", () => {
		// The hover toolbar's Edit action resolves the function through this pin. It is
		// hidden from the pin rows, never removed from the node the renderer carries.
		const before = board({
			nodes: { c1: callNode("c1", "l1") },
			layers: { l1: functionLayer("l1") },
		});
		const parsed = parse(before);
		const pin = Object.values(node(parsed, "c1").data.node.pins).find(
			(candidate) => candidate.name === "function_layer_id",
		);
		expect(parseUint8ArrayToJson(pin?.default_value)).toBe("l1");
	});

	test("does not change when board.refs churns", () => {
		// refs gain a key whenever a node type appears for the first time. No mounted
		// node reads them — the editors that do are dialogs that snapshot on open.
		const before = board();
		const first = parse(before);
		const second = parse(
			edit(before, { refs: { abc: "{}" } }),
			first.nodes,
			first.edges,
		);
		expect(contentVersion(second, "n1")).toBe(contentVersion(first, "n1"));
	});

	test("does not change when an unrelated node is added", () => {
		const before = board();
		const first = parse(before);
		const token = contentVersion(first, "n1");

		const second = parse(
			edit(before, { nodes: { ...before.nodes, n2: getNode("n2", "v1") } }),
			first.nodes,
			first.edges,
		);

		// Membership moved, so parseBoard's own reuse key must change...
		expect(node(second, "n1").data.boardDataVersion).not.toBe(
			node(first, "n1").data.boardDataVersion,
		);
		// ...but nothing the node reads did, so it must not re-render.
		expect(contentVersion(second, "n1")).toBe(token);
	});
});

/**
 * `{"type":"object","additionalProperties":true}` — `flow_like::flow::pin::OPEN_OBJECT_SCHEMA`,
 * stamped on every pin built with `Pin::set_open_schema()`.
 */
const OPEN_SCHEMA = '{"type":"object","additionalProperties":true}';
const USER_SCHEMA = '{"type":"object","properties":{"sub":{"type":"string"}}}';
const OTHER_SCHEMA = '{"type":"object","properties":{"count":{"type":"number"}}}';

const structPin = (name: string, overrides: Partial<IPin> = {}): IPin =>
	({
		id: `pin_${name}`,
		name,
		friendly_name: name,
		description: "",
		pin_type: IPinType.Output,
		data_type: IVariableType.Struct,
		value_type: IValueType.Normal,
		depends_on: [],
		connected_to: [],
		index: 0,
		...overrides,
	}) as IPin;

/**
 * A pin declaring an open shape must never veto a peer. Regression guard for the break that
 * followed `set_open_schema()` landing on `struct_in` and the whole Structs family: the pins went
 * from `schema: null` to the open marker, and the two-sided equality check in `doPinsMatch` — which
 * sits ahead of the break/make escape hatch — started rejecting every typed struct producer.
 */
describe("doPinsMatch treats an open-object schema as no schema", () => {
	test("a typed struct output connects to Break Struct's struct_in", () => {
		expect(
			doPinsMatch(
				structPin("user_context", { schema: USER_SCHEMA }),
				structPin("struct_in", {
					pin_type: IPinType.Input,
					schema: OPEN_SCHEMA,
				}),
				{},
			),
		).toBe(true);
	});

	test("an enforcing typed output still connects to struct_in", () => {
		expect(
			doPinsMatch(
				structPin("user_context", {
					schema: USER_SCHEMA,
					options: { enforce_schema: true },
				}),
				structPin("struct_in", {
					pin_type: IPinType.Input,
					schema: OPEN_SCHEMA,
				}),
				{},
			),
		).toBe(true);
	});

	test("a typed output connects to a plain 'struct' input (Get Field, Has Field)", () => {
		expect(
			doPinsMatch(
				structPin("rows", { schema: USER_SCHEMA }),
				structPin("struct", { pin_type: IPinType.Input, schema: OPEN_SCHEMA }),
				{},
			),
		).toBe(true);
	});

	test("Make Struct's open output connects to a typed struct input", () => {
		expect(
			doPinsMatch(
				structPin("struct", { schema: OPEN_SCHEMA }),
				structPin("body", { pin_type: IPinType.Input, schema: USER_SCHEMA }),
				{},
			),
		).toBe(true);
	});

	test("the open schema is recognized through a board ref", () => {
		expect(
			doPinsMatch(
				structPin("rows", { schema: USER_SCHEMA }),
				structPin("struct_in", { pin_type: IPinType.Input, schema: "ref1" }),
				{ ref1: OPEN_SCHEMA },
			),
		).toBe(true);
	});

	test("the open schema is recognized regardless of key order and whitespace", () => {
		expect(
			doPinsMatch(
				structPin("rows", { schema: USER_SCHEMA }),
				structPin("struct_in", {
					pin_type: IPinType.Input,
					schema: '{ "additionalProperties" : true , "type" : "object" }',
				}),
				{},
			),
		).toBe(true);
	});

	test("two different real schemas are still rejected", () => {
		expect(
			doPinsMatch(
				structPin("a", { schema: USER_SCHEMA }),
				structPin("b", { pin_type: IPinType.Input, schema: OTHER_SCHEMA }),
				{},
			),
		).toBe(false);
	});

	test("an object schema carrying properties is not mistaken for the open marker", () => {
		expect(
			doPinsMatch(
				structPin("a", { schema: USER_SCHEMA }),
				structPin("b", {
					pin_type: IPinType.Input,
					schema:
						'{"type":"object","additionalProperties":true,"properties":{"x":{"type":"string"}}}',
				}),
				{},
			),
		).toBe(false);
	});

	test("an enforcing output still cannot reach a shapeless plain 'struct' input", () => {
		// Pre-existing behavior, unchanged: only struct_in/struct_out get the adopt-any hatch.
		expect(
			doPinsMatch(
				structPin("user_context", {
					schema: USER_SCHEMA,
					options: { enforce_schema: true },
				}),
				structPin("struct", { pin_type: IPinType.Input, schema: OPEN_SCHEMA }),
				{},
			),
		).toBe(false);
	});

	test("a value_type mismatch still blocks the struct hatch", () => {
		expect(
			doPinsMatch(
				structPin("rows", {
					schema: USER_SCHEMA,
					value_type: IValueType.Array,
				}),
				structPin("struct_in", {
					pin_type: IPinType.Input,
					schema: OPEN_SCHEMA,
				}),
				{},
			),
		).toBe(false);
	});
});

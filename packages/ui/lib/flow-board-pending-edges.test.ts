import { describe, expect, test } from "bun:test";
import type { Connection, Edge } from "@xyflow/react";
import {
	type PendingEdgeEdit,
	edgeEditApplied,
	pendingEdgeEdit,
	wireKey,
	withPendingEdges,
} from "./flow-board-helpers";
import { parseBoard } from "./flow-board-utils";
import type { IBoard, ILayer } from "./schema/flow/board";
import type { INode } from "./schema/flow/node";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "./schema/flow/pin";

function pin(id: string, pin_type: IPinType, data_type: IVariableType): IPin {
	return {
		id,
		name: id,
		friendly_name: id,
		description: "",
		pin_type,
		data_type,
		value_type: IValueType.Normal,
		index: 0,
		connected_to: [],
		depends_on: [],
	};
}

function fixture(kind = IVariableType.String, links: [string, string][] = []) {
	const boardNodes: Record<string, INode> = {};
	const pinCache = new Map<string, [IPin, INode | ILayer, boolean]>();
	for (const id of ["a", "b", "c"]) {
		const pins = [
			pin(`${id}.out`, IPinType.Output, kind),
			pin(`${id}.in`, IPinType.Input, kind),
		];
		const node: INode = {
			id,
			name: id,
			friendly_name: id,
			description: "",
			category: "test",
			coordinates: [0, 0, 0],
			pins: Object.fromEntries(pins.map((p) => [p.id, p])),
			fn_refs: {
				fn_refs: [],
				can_reference_fns: true,
				can_be_referenced_by_fns: true,
			},
		};
		boardNodes[id] = node;
		for (const p of pins) pinCache.set(p.id, [p, node, true]);
	}
	for (const [from, to] of links) {
		boardNodes[from].pins[`${from}.out`].connected_to.push(`${to}.in`);
		boardNodes[to].pins[`${to}.in`].depends_on.push(`${from}.out`);
	}
	return { boardNodes, pinCache, refs: {} as Record<string, string> };
}

const connection = (from: string, to: string): Connection => ({
	source: from,
	target: to,
	sourceHandle: `${from}.out`,
	targetHandle: `${to}.in`,
});
const wire = (from: string, to: string): Edge => ({
	...connection(from, to),
	id: `${from}.out-${to}.in`,
});
const key = (from: string, to: string) => wireKey(connection(from, to));

function parsedEdges(nodes: Record<string, INode>): Edge[] {
	const board = {
		id: "board",
		nodes,
		comments: {},
		layers: {},
		variables: {},
		refs: {},
	} as unknown as IBoard;
	const unused = async () => {};
	return parseBoard(
		board,
		"app",
		() => {},
		() => {},
		unused,
		unused,
		new Set(),
		"default",
	).edges;
}

describe("withPendingEdges", () => {
	test("returns the parsed edges untouched when nothing is saving", () => {
		const edges = [wire("a", "b")];
		expect(withPendingEdges(edges, [])).toBe(edges);
	});

	test("draws a wire that is still saving", () => {
		const edges = [wire("a", "b")];
		const pending: PendingEdgeEdit<Edge>[] = [
			{ add: wire("b", "c"), remove: new Set() },
		];
		expect(withPendingEdges(edges, pending).map((edge) => edge.id)).toEqual([
			"a.out-b.in",
			"b.out-c.in",
		]);
	});

	test("the parsed wire replaces the pending one, whichever way its pins are listed", () => {
		const parsed: Edge = {
			id: "inverted",
			source: "c",
			target: "b",
			sourceHandle: "c.in",
			targetHandle: "b.out",
		};
		const edges = [parsed];
		const pending: PendingEdgeEdit<Edge>[] = [
			{ add: wire("b", "c"), remove: new Set() },
		];
		expect(withPendingEdges(edges, pending)).toBe(edges);
	});

	test("hides a wire that is being moved until its save lands", () => {
		const pending: PendingEdgeEdit<Edge>[] = [
			{ add: wire("a", "c"), remove: new Set([key("a", "b")]) },
		];
		expect(
			withPendingEdges([wire("a", "b")], pending).map((edge) => edge.id),
		).toEqual(["a.out-c.in"]);
	});

	test("a later drop wins over an earlier one for the same wire", () => {
		const pending: PendingEdgeEdit<Edge>[] = [
			{ remove: new Set([key("a", "b")]) },
			{ add: wire("a", "b"), remove: new Set() },
		];
		expect(
			withPendingEdges([wire("a", "b")], pending).map((edge) => edge.id),
		).toEqual(["a.out-b.in"]);
	});
});

describe("edgeEditApplied", () => {
	const edit: PendingEdgeEdit<Edge> = {
		add: wire("a", "c"),
		remove: new Set([key("a", "b")]),
	};

	test("is false while the board still shows the old wire", () => {
		expect(edgeEditApplied(edit, [wire("a", "b")])).toBe(false);
		expect(edgeEditApplied(edit, [wire("a", "b"), wire("a", "c")])).toBe(false);
	});

	test("is true once the board shows the new wire and not the old one", () => {
		expect(edgeEditApplied(edit, [wire("a", "c")])).toBe(true);
	});
});

describe("pendingEdgeEdit", () => {
	test("a new connection is the wire the saved board will draw", () => {
		const board = fixture();
		const edit = pendingEdgeEdit({
			...board,
			connection: connection("a", "b"),
			pathType: "default",
		});
		const saved = parsedEdges(
			fixture(IVariableType.String, [["a", "b"]]).boardNodes,
		);
		expect(saved).toHaveLength(1);
		expect(edit?.add).toEqual({
			...saved[0],
			data: { ...saved[0].data, reduceMotion: true },
		});
		expect([...(edit?.remove ?? [])]).toEqual([]);
	});

	test("a connection to an occupied input hides the wire it displaces", () => {
		const board = fixture(IVariableType.String, [["a", "c"]]);
		const edit = pendingEdgeEdit({
			...board,
			connection: connection("b", "c"),
		});
		expect(edit?.add?.id).toBe("b.out-c.in");
		expect([...(edit?.remove ?? [])]).toEqual([key("a", "c")]);
	});

	test("a moved wire end hides the old wire and draws the new one", () => {
		const board = fixture(IVariableType.String, [["a", "b"]]);
		const edit = pendingEdgeEdit({
			...board,
			connection: connection("a", "c"),
			oldEdge: wire("a", "b"),
		});
		expect(edit?.add?.id).toBe("a.out-c.in");
		expect([...(edit?.remove ?? [])]).toEqual([key("a", "b")]);
	});

	test("a removed wire is hidden and nothing is drawn", () => {
		const board = fixture(IVariableType.String, [["a", "b"]]);
		const edit = pendingEdgeEdit({ ...board, oldEdge: wire("a", "b") });
		expect(edit?.add).toBeUndefined();
		expect([...(edit?.remove ?? [])]).toEqual([key("a", "b")]);
	});

	test("a function reference draws its reference wire", () => {
		const board = fixture();
		const edit = pendingEdgeEdit({
			...board,
			connection: {
				source: "a",
				target: "b",
				sourceHandle: "ref_out_a",
				targetHandle: "ref_in_b",
			},
		});
		expect(edit?.add).toMatchObject({
			id: "ref_out_a-ref_in_b",
			type: "veil",
			data: { isFnRef: true },
		});
	});

	test("an existing or invalid connection shows nothing", () => {
		const board = fixture(IVariableType.String, [["a", "b"]]);
		expect(
			pendingEdgeEdit({ ...board, connection: connection("a", "b") }),
		).toBeUndefined();
		expect(
			pendingEdgeEdit({
				...board,
				connection: { ...connection("a", "b"), targetHandle: "missing" },
			}),
		).toBeUndefined();
	});
});

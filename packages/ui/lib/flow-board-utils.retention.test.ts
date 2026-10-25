import { setFlagsFromString } from "node:v8";
import { runInNewContext } from "node:vm";
import { describe, expect, test, vi } from "vitest";
import { parseBoard } from "./flow-board-utils";

// The component module drags in the editor (and its CSS), which Node cannot load.
vi.mock("../components/flow/layer-inner-node", () => ({
	InnerLayerNodeType: { INPUT: 0, RETURN: 1 },
}));
import { type IBoard, ILayerType } from "./schema/flow/board";
import type { INode } from "./schema/flow/node";

// The closure chain is V8-specific (JavaScriptCore frees it), so this runs under vitest on Node:
// `cd packages/ui && bunx vitest run lib/flow-board-utils.retention.test.ts`.
const isV8 = !process.versions.bun && Boolean(process.versions.v8);

function collectGarbage(): () => void {
	setFlagsFromString("--expose-gc");
	return runInNewContext("gc");
}

const graphNode = (id: string, hash: number, x: number): INode =>
	({
		id,
		name: "some_catalog_node",
		friendly_name: id,
		description: "",
		category: "Utils",
		coordinates: [x, 0, 0],
		hash,
		pins: {
			[`${id}-in`]: {
				id: `${id}-in`,
				name: "exec_in",
				friendly_name: "In",
				description: "",
				pin_type: "Input",
				data_type: "Execution",
				value_type: "Normal",
				index: 0,
				connected_to: [],
				depends_on: [],
				default_value: null,
			},
		},
	}) as unknown as INode;

const board = (nodes: Record<string, INode>): IBoard =>
	({
		id: "board-1",
		name: "b",
		nodes,
		variables: {},
		// Layer chips are rebuilt on every parse, so each parse used to add a link.
		layers: {
			layer: {
				id: "layer",
				name: "layer",
				type: ILayerType.Collapsed,
				nodes: {},
				pins: {},
				variables: {},
				comments: {},
				coordinates: [0, 0, 0],
				parent_id: null,
			},
		},
		comments: {},
		refs: {},
		version: [0, 0, 1],
	}) as unknown as IBoard;

type Rendered = { nodes: { id: string }[]; edges: unknown[] };

const parse = (source: IBoard, previous?: Rendered) =>
	parseBoard(
		source,
		"app-1",
		() => {},
		() => {},
		async () => {},
		async () => undefined,
		new Set<string>(),
		undefined,
		previous?.nodes,
		previous?.edges,
		undefined,
		{ current: source },
	) as Rendered;

/** Moves one node per edit, feeding each parse back in like the board does. */
function editRepeatedly(edits: number) {
	const ids = Array.from({ length: 20 }, (_, index) => `n${index}`);
	let nodes = Object.fromEntries(ids.map((id) => [id, graphNode(id, 1, 0)]));
	let parsed = parse(board(nodes));
	const replaced: WeakRef<object>[] = [];
	for (let edit = 0; edit < edits; edit++) {
		const id = ids[edit % ids.length];
		const current = parsed.nodes.find((candidate) => candidate.id === id);
		if (!current) throw new Error(`node ${id} was not rendered`);
		replaced.push(new WeakRef(current));
		nodes = { ...nodes, [id]: graphNode(id, edit + 2, edit) };
		parsed = parse(board(nodes), parsed);
	}
	return { replaced, latest: parsed };
}

describe.skipIf(!isV8)("parseBoard retention", () => {
	test("rendered nodes do not keep the parses before them alive", async () => {
		const gc = collectGarbage();
		// Run outside this async frame, which would keep its last locals across the await.
		const { replaced, latest } = editRepeatedly(60);
		await new Promise((resolve) => setTimeout(resolve, 0));
		gc();
		expect(replaced.filter((ref) => ref.deref() !== undefined)).toHaveLength(0);
		// The board keeps its current nodes; they are the root any chain would hang from.
		expect(latest.nodes).toHaveLength(21);
	});
});

import { describe, expect, test } from "bun:test";
import {
	computeFlowLayout,
	computeFlowLayoutDetailed,
	countRenderedPinRows,
	measureNodeBox,
} from "./flow-auto-layout";
import { GraphBuilder } from "./flow-layout/test-fixtures";
import { type ILayer, ILayerType } from "./schema/flow/board";
import {
	type INode,
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "./schema/flow/node";

function createPin(
	id: string,
	pinType: IPinType,
	dataType: IVariableType,
	connectedTo: string[] = [],
): IPin {
	return {
		connected_to: connectedTo,
		data_type: dataType,
		depends_on: [],
		description: id,
		friendly_name: id,
		id,
		index: 0,
		name: id,
		pin_type: pinType,
		value_type: IValueType.Normal,
	};
}

function createExecNode(options: {
	id: string;
	coordinates?: number[];
	start?: boolean;
	eventCallback?: boolean;
	fnRefs?: string[];
	dataIn?: boolean;
	dataOutTargets?: string[];
	execIn?: boolean;
	execOutTargets?: string[];
	layer?: string;
}): INode {
	const pins: Record<string, IPin> = {};
	if (options.dataIn) {
		pins[`${options.id}:value-in`] = createPin(
			`${options.id}:value-in`,
			IPinType.Input,
			IVariableType.String,
		);
	}
	if (options.dataOutTargets) {
		pins[`${options.id}:value-out`] = createPin(
			`${options.id}:value-out`,
			IPinType.Output,
			IVariableType.String,
			options.dataOutTargets,
		);
	}
	if (options.execIn) {
		pins[`${options.id}:exec-in`] = createPin(
			`${options.id}:exec-in`,
			IPinType.Input,
			IVariableType.Execution,
		);
	}
	if (options.execOutTargets) {
		pins[`${options.id}:exec-out`] = createPin(
			`${options.id}:exec-out`,
			IPinType.Output,
			IVariableType.Execution,
			options.execOutTargets,
		);
	}

	return {
		id: options.id,
		category: "Tests/Layout",
		coordinates: options.coordinates ?? [0, 0, 0],
		description: options.id,
		event_callback: options.eventCallback ?? false,
		friendly_name: options.id,
		fn_refs: options.fnRefs
			? {
					can_be_referenced_by_fns: false,
					can_reference_fns: options.fnRefs.length > 0,
					fn_refs: options.fnRefs,
				}
			: null,
		name: options.id,
		layer: options.layer,
		pins,
		start: options.start ?? false,
	};
}

function createPureNode(options: {
	id: string;
	coordinates?: number[];
	inputTargets?: string[];
	outputTargets?: string[];
	layer?: string;
}): INode {
	const pins: Record<string, IPin> = {
		[`${options.id}:value-in`]: createPin(
			`${options.id}:value-in`,
			IPinType.Input,
			IVariableType.String,
		),
		[`${options.id}:value-out`]: createPin(
			`${options.id}:value-out`,
			IPinType.Output,
			IVariableType.String,
			options.outputTargets ?? [],
		),
	};

	if (options.inputTargets) {
		pins[`${options.id}:value-in`].connected_to = options.inputTargets;
	}

	return {
		id: options.id,
		category: "Tests/Layout",
		coordinates: options.coordinates ?? [0, 0, 0],
		description: options.id,
		event_callback: false,
		friendly_name: options.id,
		fn_refs: null,
		name: options.id,
		layer: options.layer,
		pins,
		start: false,
	};
}

function createLayer(options: {
	id: string;
	parentId?: string;
	nodes?: Record<string, INode>;
	coordinates?: number[];
}): ILayer {
	return {
		comments: {},
		coordinates: options.coordinates ?? [0, 0, 0],
		id: options.id,
		name: options.id,
		nodes: options.nodes ?? {},
		parent_id: options.parentId,
		pins: {},
		type: ILayerType.Collapsed,
		variables: {},
	};
}

describe("computeFlowLayout", () => {
	test("keeps execution fan-out in the same branch column", () => {
		const left = createExecNode({
			id: "left",
			coordinates: [300, -100, 0],
			execIn: true,
		});
		const right = createExecNode({
			id: "right",
			coordinates: [300, 120, 0],
			execIn: true,
		});
		const root = createExecNode({
			id: "root",
			coordinates: [0, 0, 0],
			start: true,
			execOutTargets: ["left:exec-in", "right:exec-in"],
		});

		const positions = computeFlowLayout({
			layerNodes: [root, left, right],
			layerEntities: [],
			currentLayer: undefined,
		});

		const rootPos = positions.get("root");
		const leftPos = positions.get("left");
		const rightPos = positions.get("right");
		expect(rootPos).toBeDefined();
		expect(leftPos).toBeDefined();
		expect(rightPos).toBeDefined();
		expect(leftPos?.[0]).toBeGreaterThan(rootPos?.[0] ?? 0);
		expect(rightPos?.[0]).toBeGreaterThan(rootPos?.[0] ?? 0);
		expect(leftPos?.[0]).toBe(rightPos?.[0]);
		expect(leftPos?.[1]).not.toBe(rightPos?.[1]);
	});

	test("resolves fn-ref targets through descendant layers of a visible group", () => {
		const nestedTarget = createExecNode({
			id: "nested-target",
			coordinates: [50, 50, 0],
			eventCallback: true,
			layer: "nested-layer",
		});
		const source = createExecNode({
			id: "source",
			coordinates: [0, 0, 0],
			start: true,
			fnRefs: [nestedTarget.id],
		});
		const otherRoot = createExecNode({
			id: "other-root",
			coordinates: [0, 500, 0],
			start: true,
		});
		const visibleEntityId = "event-group";

		const boardLayers: Record<string, ILayer> = {
			[visibleEntityId]: createLayer({
				id: visibleEntityId,
				coordinates: [600, 0, 0],
			}),
			"nested-layer": createLayer({
				id: "nested-layer",
				parentId: visibleEntityId,
				nodes: { [nestedTarget.id]: nestedTarget },
				coordinates: [700, 80, 0],
			}),
		};

		const positions = computeFlowLayout({
			layerNodes: [source, otherRoot],
			layerEntities: [{ id: visibleEntityId, coordinates: [600, 0, 0] }],
			boardLayers,
			currentLayer: undefined,
		});

		const sourcePos = positions.get(source.id);
		const otherRootPos = positions.get(otherRoot.id);
		const visibleEntityPos = positions.get(visibleEntityId);
		expect(sourcePos).toBeDefined();
		expect(otherRootPos).toBeDefined();
		expect(visibleEntityPos).toBeDefined();
		expect(visibleEntityPos?.[0]).toBe(sourcePos?.[0]);
		expect(visibleEntityPos?.[1] ?? 0).toBeLessThan(otherRootPos?.[1] ?? 0);
	});

	test("keeps connected event callbacks in the parent event group", () => {
		const callback = createExecNode({
			id: "callback",
			coordinates: [300, 200, 0],
			eventCallback: true,
			execIn: true,
			execOutTargets: ["tail:exec-in"],
		});
		const tail = createExecNode({
			id: "tail",
			coordinates: [600, 220, 0],
			execIn: true,
		});
		const root = createExecNode({
			id: "root",
			coordinates: [0, 0, 0],
			start: true,
			execOutTargets: ["callback:exec-in"],
		});
		const otherRoot = createExecNode({
			id: "other-root",
			coordinates: [0, 500, 0],
			start: true,
		});

		const positions = computeFlowLayout({
			layerNodes: [root, callback, tail, otherRoot],
			layerEntities: [],
			currentLayer: undefined,
		});

		const rootPos = positions.get(root.id);
		const callbackPos = positions.get(callback.id);
		const tailPos = positions.get(tail.id);
		const otherRootPos = positions.get(otherRoot.id);

		expect(rootPos).toBeDefined();
		expect(callbackPos).toBeDefined();
		expect(tailPos).toBeDefined();
		expect(otherRootPos).toBeDefined();
		expect(callbackPos?.[0]).toBeGreaterThan(rootPos?.[0] ?? 0);
		expect(callbackPos?.[1]).toBe(rootPos?.[1]);
		expect(tailPos?.[0]).toBeGreaterThan(callbackPos?.[0] ?? 0);
		expect(otherRootPos?.[1] ?? 0).toBeGreaterThan(callbackPos?.[1] ?? 0);
	});

	test("spreads pure dependency chains across columns instead of stacking them", () => {
		const sink = createExecNode({
			id: "sink",
			coordinates: [600, 0, 0],
			dataIn: true,
			execIn: true,
		});
		const root = createExecNode({
			id: "root",
			coordinates: [0, 0, 0],
			start: true,
			execOutTargets: ["sink:exec-in"],
		});
		const formatter = createPureNode({
			id: "formatter",
			coordinates: [300, 40, 0],
			outputTargets: ["sink:value-in"],
		});
		const source = createPureNode({
			id: "source",
			coordinates: [120, 20, 0],
			outputTargets: ["formatter:value-in"],
		});

		const positions = computeFlowLayout({
			layerNodes: [root, sink, formatter, source],
			layerEntities: [],
			currentLayer: undefined,
		});

		const sinkPos = positions.get(sink.id);
		const formatterPos = positions.get(formatter.id);
		const sourcePos = positions.get(source.id);

		expect(sinkPos).toBeDefined();
		expect(formatterPos).toBeDefined();
		expect(sourcePos).toBeDefined();
		expect(formatterPos?.[0]).toBeLessThan(sinkPos?.[0] ?? 0);
		expect(sourcePos?.[0]).toBeLessThan(formatterPos?.[0] ?? 0);
		// Pure nodes hang in a band strictly below the exec lane they feed.
		expect(formatterPos?.[1]).toBeGreaterThan(sinkPos?.[1] ?? 0);
		expect(
			Math.abs((sourcePos?.[1] ?? 0) - (formatterPos?.[1] ?? 0)),
		).toBeLessThan(150);
	});

	test("keeps inline pure nodes off the executing lane", () => {
		const sink = createExecNode({
			id: "sink-inline",
			coordinates: [600, 0, 0],
			dataIn: true,
			execIn: true,
		});
		const formatter = createPureNode({
			id: "formatter-inline",
			coordinates: [220, -40, 0],
			outputTargets: ["sink-inline:value-in"],
		});
		const root = createExecNode({
			id: "root-inline",
			coordinates: [0, 0, 0],
			start: true,
			dataOutTargets: ["formatter-inline:value-in"],
			execOutTargets: ["sink-inline:exec-in"],
		});

		const positions = computeFlowLayout({
			layerNodes: [root, sink, formatter],
			layerEntities: [],
			currentLayer: undefined,
		});

		const rootPos = positions.get(root.id);
		const formatterPos = positions.get(formatter.id);

		expect(rootPos).toBeDefined();
		expect(formatterPos).toBeDefined();
		expect(formatterPos?.[0]).toBeGreaterThan(rootPos?.[0] ?? 0);
		// The pure node sits off the exec lane, below it.
		expect(formatterPos?.[1]).toBeGreaterThan(rootPos?.[1] ?? 0);
	});
});

describe("layout invariants", () => {
	test("renders Branch True above False on an unarranged board", () => {
		const graph = new GraphBuilder();
		// Ids are chosen so alphabetical order contradicts pin order: if ordering
		// ever falls back to id comparison, this test fails.
		graph.exec("zeta-evt", { execIn: false, start: true });
		graph.exec("alpha-branch", { execOuts: ["true", "false"] });
		graph.exec("zeta-true-path");
		graph.exec("alpha-false-path");
		graph.execLink("zeta-evt", "alpha-branch");
		graph.connect("alpha-branch:true", "zeta-true-path:exec-in");
		graph.connect("alpha-branch:false", "alpha-false-path:exec-in");

		const positions = computeFlowLayout(graph.build());
		expect(positions.get("zeta-true-path")?.[1] ?? 0).toBeLessThan(
			positions.get("alpha-false-path")?.[1] ?? 0,
		);
	});

	test("keeps a single-in single-out chain on one straight line", () => {
		const graph = new GraphBuilder();
		graph.exec("evt", { execIn: false, start: true });
		let previous = "evt";
		for (let i = 0; i < 5; i++) {
			// Varying pin counts change node heights; the wire must stay flat.
			graph.exec(`step-${i}`, { dataIns: i, dataOuts: i * 2 });
			graph.execLink(previous, `step-${i}`);
			previous = `step-${i}`;
		}

		const positions = computeFlowLayout(graph.build());
		const ys = ["evt", "step-0", "step-1", "step-2", "step-3", "step-4"].map(
			(id) => positions.get(id)?.[1],
		);
		expect(new Set(ys).size).toBe(1);
	});

	test("never places a pure node right of anything it feeds", () => {
		const graph = new GraphBuilder();
		graph.exec("evt", { execIn: false, start: true });
		graph.exec("consumer", { dataIns: 1 });
		graph.pure("producer");
		graph.pure("upstream");
		graph.execLink("evt", "consumer");
		graph.dataLink("upstream", "producer");
		graph.connect("producer:out-0", "consumer:in-0");

		const { positions, diagnostics } = computeFlowLayoutDetailed(graph.build());
		const columnOf = (id: string) => diagnostics.columns.get(id) ?? 0;
		expect(columnOf("upstream")).toBeLessThan(columnOf("producer"));
		expect(columnOf("producer")).toBeLessThan(columnOf("consumer"));
		expect(positions.get("producer")?.[0] ?? 0).toBeLessThan(
			positions.get("consumer")?.[0] ?? 0,
		);
	});

	test("leaves unselected nodes untouched when scoped to a selection", () => {
		const graph = new GraphBuilder();
		graph.exec("evt", { execIn: false, start: true, coordinates: [0, 0, 0] });
		graph.exec("a", { coordinates: [900, 40, 0] });
		graph.exec("b", { coordinates: [1800, 900, 0] });
		graph.exec("untouched", { coordinates: [-4000, -4000, 0] });
		graph.execLink("evt", "a");
		graph.execLink("a", "b");

		const positions = computeFlowLayout(
			graph.build({ only: new Set(["evt", "a", "b"]) }),
		);
		expect(positions.has("untouched")).toBe(false);
		expect(positions.size).toBe(3);
	});

	test("keeps a scoped layout clear of the nodes it did not lay out", () => {
		const graph = new GraphBuilder();
		graph.exec("evt", { execIn: false, start: true, coordinates: [0, 0, 0] });
		graph.exec("a", { coordinates: [200, 0, 0] });
		graph.exec("b", { coordinates: [400, 0, 0] });
		// Sits exactly where the scoped result would otherwise land.
		graph.exec("blocker", { coordinates: [200, 0, 0] });

		const only = new Set(["evt", "a", "b"]);
		const obstacles = [{ x: 200, y: 0, width: 150, height: 58 }];
		const first = computeFlowLayout(graph.build({ only, obstacles }));

		for (const id of only) {
			const position = first.get(id);
			expect(position).toBeDefined();
			const overlapsBlocker =
				(position?.[0] ?? 0) < 350 &&
				(position?.[0] ?? 0) + 150 > 200 &&
				(position?.[1] ?? 0) < 58 &&
				(position?.[1] ?? 0) + 58 > 0;
			expect(overlapsBlocker).toBe(false);
		}

		// Pushing clear is a rigid translation, so it must still be a fixed point.
		const second = computeFlowLayout(
			graph.build({
				only,
				obstacles,
				layerNodes: [...graph.nodes.values()].map((node) => {
					const next = first.get(node.id);
					return next ? { ...node, coordinates: [next[0], next[1], 0] } : node;
				}),
			}),
		);
		expect([...second.entries()].sort()).toEqual([...first.entries()].sort());
	});

	test("anchors a layer body on its own boundary nodes", () => {
		// Mirrors what flow-board.tsx synthesises for an open layer: `-input`
		// carries the layer's Input pins inverted to Output, `-return` the reverse.
		const graph = new GraphBuilder();
		graph.exec("L-input", {
			execIn: false,
			dataOuts: 1,
			start: true,
			coordinates: [-200, 0, 0],
		});
		graph.exec("body-a", { dataIns: 1, dataOuts: 1, coordinates: [0, 0, 0] });
		graph.exec("body-b", { dataIns: 1, dataOuts: 1, coordinates: [300, 0, 0] });
		graph.exec("L-return", {
			execOuts: 0,
			dataIns: 1,
			coordinates: [900, 0, 0],
		});
		graph.execLink("L-input", "body-a");
		graph.execLink("body-a", "body-b");
		graph.execLink("body-b", "L-return");
		graph.dataLink("L-input", "body-a");
		graph.dataLink("body-a", "body-b");
		graph.dataLink("body-b", "L-return");

		const { positions, diagnostics } = computeFlowLayoutDetailed(graph.build());
		const columns = [...diagnostics.columns.values()];
		expect(diagnostics.columns.get("L-input")).toBe(0);
		expect(diagnostics.columns.get("L-return")).toBe(Math.max(...columns));
		// The boundary the user entered through does not move.
		expect(positions.get("L-input")).toEqual([-200, 0]);
	});

	test("moves a comment with the nodes it covers", () => {
		const graph = new GraphBuilder();
		graph.exec("evt", {
			execIn: false,
			start: true,
			coordinates: [1000, 1000, 0],
		});
		graph.exec("step", { coordinates: [1200, 1000, 0] });
		graph.execLink("evt", "step");

		const { commentPositions } = computeFlowLayoutDetailed(
			graph.build({
				comments: [
					{ id: "note", x: 960, y: 960, width: 500, height: 200 },
					{ id: "far-away", x: -9000, y: -9000, width: 100, height: 100 },
				],
			}),
		);
		// Anchored on the start node, which does not move, so the comment holds
		// its relative offset.
		expect(commentPositions.get("note")).toEqual([960, 960]);
		expect(commentPositions.has("far-away")).toBe(false);
	});

	test("measures node boxes from the renderer's own formula", () => {
		const graph = new GraphBuilder();
		const zero = graph.exec("zero", { execIn: false, execOuts: 0 });
		const four = graph.exec("four", { dataIns: 3, dataOuts: 0 });

		expect(measureNodeBox(zero)).toEqual({ width: 150, height: 28 });
		// 1 exec-in + 3 data-in = 4 rendered rows.
		expect(countRenderedPinRows(four)).toBe(4);
		expect(measureNodeBox(four)).toEqual({ width: 150, height: 4 * 15 + 28 });
	});

	test("counts the synthetic add-pin row that duplicate pin names render", () => {
		const graph = new GraphBuilder();
		const node = graph.exec("dynamic", { execIn: false, execOuts: 0 });
		for (let i = 0; i < 3; i++) {
			node.pins[`dynamic:item-${i}`] = {
				connected_to: [],
				data_type: IVariableType.String,
				depends_on: [],
				description: "item",
				friendly_name: "item",
				id: `dynamic:item-${i}`,
				index: i + 1,
				name: "item",
				pin_type: IPinType.Input,
				value_type: IValueType.Normal,
			};
		}
		// 3 pins sharing a name render an extra "add pin" action row.
		expect(countRenderedPinRows(node)).toBe(4);
	});
});

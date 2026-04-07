import { describe, expect, test } from "bun:test";
import { computeFlowLayout } from "./flow-auto-layout";
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
		expect(formatterPos?.[1]).not.toBe(sinkPos?.[1]);
		expect(
			Math.abs((formatterPos?.[1] ?? 0) - (sinkPos?.[1] ?? 0)),
		).toBeGreaterThanOrEqual(100);
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
		expect(formatterPos?.[1]).not.toBe(rootPos?.[1]);
		expect(
			Math.abs((formatterPos?.[1] ?? 0) - (rootPos?.[1] ?? 0)),
		).toBeGreaterThanOrEqual(100);
	});
});

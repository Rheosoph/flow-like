import type { ILayer } from "../schema/flow/board";
import { ILayerType } from "../schema/flow/board";
import type { INode, IPin } from "../schema/flow/node";
import { IPinType, IValueType, IVariableType } from "../schema/flow/node";
import type { AutoLayoutInput } from "./types";

export interface Scenario {
	name: string;
	input: AutoLayoutInput;
}

function pin(
	id: string,
	name: string,
	pinType: IPinType,
	dataType: IVariableType,
	index: number,
	connectedTo: string[] = [],
): IPin {
	return {
		connected_to: connectedTo,
		data_type: dataType,
		depends_on: [],
		description: id,
		friendly_name: name,
		id,
		index,
		name,
		pin_type: pinType,
		value_type: IValueType.Normal,
	};
}

export interface ExecOptions {
	execIn?: boolean;
	execOuts?: number | string[];
	dataIns?: number;
	dataOuts?: number;
	start?: boolean;
	fnRefs?: string[];
	coordinates?: number[];
	layer?: string;
}

/**
 * Builds nodes the way the runtime does: `PinIndicesCleanup` in packages/core
 * re-indexes inputs and outputs separately starting at 1, so a node's exec-in
 * and exec-out both land on row 1 and a straight chain stays flat.
 */
export class GraphBuilder {
	readonly nodes = new Map<string, INode>();
	readonly entities: Array<{ id: string; coordinates: number[] }> = [];
	readonly boardLayers: Record<string, ILayer> = {};

	private base(id: string, coordinates?: number[]): INode {
		return {
			id,
			category: "Tests/Layout",
			coordinates: coordinates ?? [0, 0, 0],
			description: id,
			event_callback: false,
			friendly_name: id,
			fn_refs: null,
			name: id,
			pins: {},
			start: false,
		};
	}

	exec(id: string, options: ExecOptions = {}): INode {
		const node = this.base(id, options.coordinates);
		node.layer = options.layer;
		let inIndex = 1;
		let outIndex = 1;

		if (options.execIn !== false) {
			const pinId = `${id}:exec-in`;
			node.pins[pinId] = pin(
				pinId,
				"exec_in",
				IPinType.Input,
				IVariableType.Execution,
				inIndex++,
			);
		}
		const outNames =
			typeof options.execOuts === "object"
				? options.execOuts
				: Array.from(
						{ length: options.execOuts ?? 1 },
						(_, index) => `exec-out-${index}`,
					);
		for (const outName of outNames) {
			const pinId = `${id}:${outName}`;
			node.pins[pinId] = pin(
				pinId,
				outName,
				IPinType.Output,
				IVariableType.Execution,
				outIndex++,
			);
		}
		for (let i = 0; i < (options.dataIns ?? 0); i++) {
			const pinId = `${id}:in-${i}`;
			node.pins[pinId] = pin(
				pinId,
				`in_${i}`,
				IPinType.Input,
				IVariableType.String,
				inIndex++,
			);
		}
		for (let i = 0; i < (options.dataOuts ?? 0); i++) {
			const pinId = `${id}:out-${i}`;
			node.pins[pinId] = pin(
				pinId,
				`out_${i}`,
				IPinType.Output,
				IVariableType.String,
				outIndex++,
			);
		}

		node.start = options.start ?? false;
		if (options.fnRefs) {
			node.fn_refs = {
				can_be_referenced_by_fns: false,
				can_reference_fns: true,
				fn_refs: options.fnRefs,
			};
		}
		this.nodes.set(id, node);
		return node;
	}

	pure(
		id: string,
		options: {
			dataIns?: number;
			dataOuts?: number;
			coordinates?: number[];
		} = {},
	): INode {
		const node = this.base(id, options.coordinates);
		let inIndex = 1;
		let outIndex = 1;
		for (let i = 0; i < (options.dataIns ?? 1); i++) {
			const pinId = `${id}:in-${i}`;
			node.pins[pinId] = pin(
				pinId,
				`in_${i}`,
				IPinType.Input,
				IVariableType.String,
				inIndex++,
			);
		}
		for (let i = 0; i < (options.dataOuts ?? 1); i++) {
			const pinId = `${id}:out-${i}`;
			node.pins[pinId] = pin(
				pinId,
				`out_${i}`,
				IPinType.Output,
				IVariableType.String,
				outIndex++,
			);
		}
		this.nodes.set(id, node);
		return node;
	}

	layer(
		id: string,
		options: { parentId?: string; coordinates?: number[] } = {},
	) {
		this.boardLayers[id] = {
			comments: {},
			coordinates: options.coordinates ?? [0, 0, 0],
			id,
			name: id,
			nodes: {},
			parent_id: options.parentId,
			pins: {},
			type: ILayerType.Function,
			variables: {},
		};
		if (!options.parentId) {
			this.entities.push({
				id,
				coordinates: options.coordinates ?? [0, 0, 0],
			});
		}
	}

	connect(fromPinId: string, toPinId: string) {
		for (const node of this.nodes.values()) {
			const source = node.pins[fromPinId];
			if (source) {
				source.connected_to = [...source.connected_to, toPinId];
				return;
			}
		}
		throw new Error(`unknown source pin ${fromPinId}`);
	}

	execLink(from: string, to: string, outName = "exec-out-0") {
		this.connect(`${from}:${outName}`, `${to}:exec-in`);
	}

	dataLink(from: string, to: string, outIndex = 0, inIndex = 0) {
		this.connect(`${from}:out-${outIndex}`, `${to}:in-${inIndex}`);
	}

	build(overrides: Partial<AutoLayoutInput> = {}): AutoLayoutInput {
		return {
			layerNodes: [...this.nodes.values()],
			layerEntities: this.entities,
			boardLayers: this.boardLayers,
			currentLayer: undefined,
			...overrides,
		};
	}
}

function linearChain(length: number): Scenario {
	const graph = new GraphBuilder();
	graph.exec("evt", { execIn: false, start: true });
	let previous = "evt";
	for (let i = 0; i < length; i++) {
		graph.exec(`step-${i}`);
		graph.execLink(previous, `step-${i}`);
		previous = `step-${i}`;
	}
	return { name: "linear-chain", input: graph.build() };
}

function branchDiamond(): Scenario {
	const graph = new GraphBuilder();
	graph.exec("evt", { execIn: false, start: true });
	graph.exec("branch", { execOuts: ["true", "false"] });
	graph.exec("on-true");
	graph.exec("on-false");
	graph.exec("merge");
	graph.execLink("evt", "branch");
	graph.connect("branch:true", "on-true:exec-in");
	graph.connect("branch:false", "on-false:exec-in");
	graph.execLink("on-true", "merge");
	graph.execLink("on-false", "merge");
	return { name: "branch-diamond", input: graph.build() };
}

function execCycleDescendingY(): Scenario {
	const graph = new GraphBuilder();
	graph.exec("start", { execIn: false, start: true, coordinates: [0, 500, 0] });
	graph.exec("a", { execOuts: 1, coordinates: [0, 400, 0] });
	graph.exec("b", { execOuts: 2, coordinates: [0, 300, 0] });
	graph.exec("c", { coordinates: [0, 200, 0] });
	graph.exec("d", { coordinates: [0, 100, 0] });
	graph.execLink("start", "a");
	graph.execLink("a", "b");
	graph.connect("b:exec-out-0", "c:exec-in");
	graph.connect("b:exec-out-1", "a:exec-in");
	graph.execLink("c", "d");
	return { name: "exec-cycle-descending-y", input: graph.build() };
}

function backEdgeIntoStart(): Scenario {
	const graph = new GraphBuilder();
	graph.exec("start", { execIn: true, start: true });
	graph.exec("a");
	graph.exec("b");
	graph.execLink("start", "a");
	graph.execLink("a", "b");
	graph.execLink("b", "start");
	return { name: "back-edge-into-start", input: graph.build() };
}

function pureDataCycle(): Scenario {
	const graph = new GraphBuilder();
	graph.exec("start", { execIn: false, start: true, dataOuts: 1 });
	graph.exec("sink", { dataIns: 1 });
	graph.pure("p1");
	graph.pure("p2");
	graph.execLink("start", "sink");
	graph.connect("start:out-0", "p1:in-0");
	graph.dataLink("p1", "p2");
	graph.dataLink("p2", "p1");
	graph.connect("p2:out-0", "sink:in-0");
	return { name: "pure-data-cycle", input: graph.build() };
}

function tallSiblings(): Scenario {
	const graph = new GraphBuilder();
	graph.exec("evt", { execIn: false, start: true });
	graph.exec("branch", { execOuts: ["true", "false"] });
	graph.exec("tall-a", { dataIns: 24, dataOuts: 24 });
	graph.exec("tall-b", { dataIns: 24, dataOuts: 24 });
	graph.execLink("evt", "branch");
	graph.connect("branch:true", "tall-a:exec-in");
	graph.connect("branch:false", "tall-b:exec-in");
	return { name: "tall-siblings", input: graph.build() };
}

function deepPureTree(): Scenario {
	const graph = new GraphBuilder();
	graph.exec("evt", { execIn: false, start: true });
	graph.exec("sink", { dataIns: 6 });
	graph.execLink("evt", "sink");
	for (let i = 0; i < 6; i++) {
		graph.pure(`mid-${i}`, { dataIns: 2 });
		graph.connect(`mid-${i}:out-0`, `sink:in-${i}`);
		for (let j = 0; j < 2; j++) {
			graph.pure(`leaf-${i}-${j}`);
			graph.connect(`leaf-${i}-${j}:out-0`, `mid-${i}:in-${j}`);
		}
	}
	return { name: "deep-pure-tree", input: graph.build() };
}

function sharedPureThreeConsumers(): Scenario {
	const graph = new GraphBuilder();
	graph.exec("evt", { execIn: false, start: true });
	graph.pure("shared");
	let previous = "evt";
	for (let i = 0; i < 3; i++) {
		graph.exec(`consumer-${i}`, { dataIns: 1 });
		graph.execLink(previous, `consumer-${i}`);
		graph.connect("shared:out-0", `consumer-${i}:in-0`);
		previous = `consumer-${i}`;
	}
	return { name: "shared-pure-three-consumers", input: graph.build() };
}

function fanOut(width: number): Scenario {
	const graph = new GraphBuilder();
	graph.exec("evt", { execIn: false, start: true });
	graph.exec("hub", {
		execOuts: Array.from({ length: width }, (_, i) => `out-${i}`),
	});
	graph.execLink("evt", "hub");
	for (let i = 0; i < width; i++) {
		graph.exec(`sink-${i}`);
		graph.connect(`hub:out-${i}`, `sink-${i}:exec-in`);
	}
	return { name: `fanout-${width}`, input: graph.build() };
}

function fnRefsTwoCallNodes(): Scenario {
	const graph = new GraphBuilder();
	graph.layer("fn-a", { coordinates: [900, 0, 0] });
	graph.layer("fn-b", { coordinates: [900, 400, 0] });
	graph.exec("evt", { execIn: false, start: true });
	graph.exec("call-a", { fnRefs: ["fn-a"] });
	graph.exec("call-b", { fnRefs: ["fn-b"] });
	graph.execLink("evt", "call-a");
	graph.execLink("call-a", "call-b");
	return { name: "fnrefs-two-call-nodes", input: graph.build() };
}

function noStartChain(order: number[]): Scenario {
	const graph = new GraphBuilder();
	for (let i = 1; i <= 4; i++) graph.exec(`n${i}`);
	for (let i = 1; i < 4; i++) graph.execLink(`n${i}`, `n${i + 1}`);
	const all = [...graph.nodes.values()];
	return {
		name: `no-start-chain-${order.join("")}`,
		input: graph.build({ layerNodes: order.map((index) => all[index - 1]) }),
	};
}

function fiveEventGroups(): Scenario {
	const graph = new GraphBuilder();
	for (let g = 0; g < 5; g++) {
		graph.exec(`evt-${g}`, {
			execIn: false,
			start: true,
			coordinates: [0, g * 400, 0],
		});
		let previous = `evt-${g}`;
		for (let i = 0; i < 4; i++) {
			graph.exec(`g${g}-step-${i}`, {
				coordinates: [(i + 1) * 300, g * 400, 0],
			});
			graph.execLink(previous, `g${g}-step-${i}`);
			previous = `g${g}-step-${i}`;
		}
	}
	return { name: "five-event-groups", input: graph.build() };
}

function convergingEvents(): Scenario {
	const graph = new GraphBuilder();
	graph.exec("evt-a", { execIn: false, start: true, coordinates: [0, 0, 0] });
	graph.exec("evt-b", { execIn: false, start: true, coordinates: [0, 400, 0] });
	graph.exec("pre-a");
	graph.exec("pre-b");
	graph.exec("shared-1");
	graph.exec("shared-2");
	graph.execLink("evt-a", "pre-a");
	graph.execLink("evt-b", "pre-b");
	graph.execLink("pre-a", "shared-1");
	graph.execLink("pre-b", "shared-1");
	graph.execLink("shared-1", "shared-2");
	return { name: "converging-events", input: graph.build() };
}

function orphans(count: number): Scenario {
	const graph = new GraphBuilder();
	for (let i = 0; i < count; i++) {
		if (i % 2 === 0) graph.exec(`orphan-exec-${i}`);
		else graph.pure(`orphan-pure-${i}`);
	}
	return { name: `orphans-${count}`, input: graph.build() };
}

function pureOnlyIsland(): Scenario {
	const graph = new GraphBuilder();
	for (let i = 0; i < 6; i++) graph.pure(`p-${i}`);
	for (let i = 0; i < 5; i++) graph.dataLink(`p-${i}`, `p-${i + 1}`);
	return { name: "pure-only-island", input: graph.build() };
}

function denseDataMesh(): Scenario {
	const graph = new GraphBuilder();
	graph.exec("evt", { execIn: false, start: true });
	let previous = "evt";
	for (let i = 0; i < 8; i++) {
		graph.exec(`sink-${i}`, { dataIns: 4 });
		graph.execLink(previous, `sink-${i}`);
		previous = `sink-${i}`;
	}
	for (let i = 0; i < 8; i++) {
		graph.pure(`mesh-${i}`, { dataOuts: 1 });
		for (let j = 0; j < 4; j++) {
			graph.connect(`mesh-${i}:out-0`, `sink-${(i + j) % 8}:in-${j}`);
		}
	}
	return { name: "dense-data-mesh", input: graph.build() };
}

function longBackEdge(): Scenario {
	const graph = new GraphBuilder();
	graph.exec("evt", { execIn: false, start: true });
	let previous = "evt";
	for (let i = 0; i < 14; i++) {
		graph.exec(`n-${i}`, { execOuts: i === 13 ? 1 : 1 });
		graph.execLink(previous, `n-${i}`);
		previous = `n-${i}`;
	}
	graph.execLink("n-13", "n-0");
	return { name: "long-back-edge", input: graph.build() };
}

function largeMixed(target: number): Scenario {
	const graph = new GraphBuilder();
	let created = 0;
	let group = 0;
	while (created < target) {
		graph.exec(`g${group}-evt`, { execIn: false, start: true });
		created++;
		let previous = `g${group}-evt`;
		for (let i = 0; i < 6 && created < target; i++) {
			graph.exec(`g${group}-step-${i}`, { dataIns: 2 });
			graph.execLink(previous, `g${group}-step-${i}`);
			previous = `g${group}-step-${i}`;
			created++;
			for (let p = 0; p < 2 && created < target; p++) {
				graph.pure(`g${group}-p-${i}-${p}`);
				graph.connect(
					`g${group}-p-${i}-${p}:out-0`,
					`g${group}-step-${i}:in-${p}`,
				);
				created++;
			}
		}
		if (group % 3 === 0) graph.execLink(previous, `g${group}-evt`);
		group++;
	}
	return { name: `large-${target}-mixed`, input: graph.build() };
}

function allZeroCoordinates(): Scenario {
	const graph = new GraphBuilder();
	graph.exec("zeta-evt", { execIn: false, start: true });
	graph.exec("alpha-branch", { execOuts: ["true", "false"] });
	graph.exec("zeta-true-path");
	graph.exec("alpha-false-path");
	graph.execLink("zeta-evt", "alpha-branch");
	graph.connect("alpha-branch:true", "zeta-true-path:exec-in");
	graph.connect("alpha-branch:false", "alpha-false-path:exec-in");
	return { name: "all-zero-coordinates", input: graph.build() };
}

function selfLoop(): Scenario {
	const graph = new GraphBuilder();
	graph.exec("evt", { execIn: false, start: true });
	graph.exec("self", { execOuts: 1 });
	graph.execLink("evt", "self");
	graph.execLink("self", "self");
	return { name: "self-loop", input: graph.build() };
}

export function allScenarios(): Scenario[] {
	return [
		linearChain(20),
		branchDiamond(),
		execCycleDescendingY(),
		backEdgeIntoStart(),
		pureDataCycle(),
		tallSiblings(),
		deepPureTree(),
		sharedPureThreeConsumers(),
		fanOut(8),
		fanOut(60),
		fnRefsTwoCallNodes(),
		noStartChain([1, 2, 3, 4]),
		noStartChain([3, 1, 4, 2]),
		noStartChain([4, 3, 2, 1]),
		noStartChain([2, 4, 1, 3]),
		fiveEventGroups(),
		convergingEvents(),
		orphans(20),
		pureOnlyIsland(),
		denseDataMesh(),
		longBackEdge(),
		allZeroCoordinates(),
		selfLoop(),
		largeMixed(300),
	];
}

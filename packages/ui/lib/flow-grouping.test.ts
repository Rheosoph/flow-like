import { describe, expect, test } from "bun:test";
import {
	evaluateGroupCandidate,
	evaluateGroupCandidates,
	findGroupSuggestions,
} from "./flow-grouping";
import { GraphBuilder } from "./flow-layout/test-fixtures";
import type { IBoard, ILayer } from "./schema/flow/board";
import { ILayerType } from "./schema/flow/board";
import type { INode } from "./schema/flow/node";

function boardOf(graph: GraphBuilder): IBoard {
	return {
		nodes: Object.fromEntries(graph.nodes),
		layers: {},
		comments: {},
	} as IBoard;
}

function diamond(prefix = "") {
	const graph = new GraphBuilder();
	const id = (value: string) => prefix + value;
	graph.exec(id("event"), { start: true, execIn: false });
	graph.exec(id("prepare"), { execOuts: 2, coordinates: [230, 0] });
	graph.exec(id("left"), { coordinates: [460, 0] });
	graph.exec(id("right"), { coordinates: [460, 100] });
	graph.exec(id("combine"), { coordinates: [690, 0] });
	graph.exec(id("sink"), { coordinates: [920, 0] });
	graph.execLink(id("event"), id("prepare"));
	graph.execLink(id("prepare"), id("left"));
	graph.execLink(id("prepare"), id("right"), "exec-out-1");
	graph.execLink(id("left"), id("combine"));
	graph.execLink(id("right"), id("combine"));
	graph.execLink(id("combine"), id("sink"));
	return {
		graph,
		board: boardOf(graph),
		members: ["combine", "left", "prepare", "right"].map(id),
	};
}

/** A long board of branch/merge clusters chained by exec wires, in one row. */
function chainedClusters(count: number): IBoard {
	const graph = new GraphBuilder();
	const pins = new Map<string, INode["pins"][string]>();
	const add = (node: INode) => {
		for (const entry of Object.values(node.pins)) pins.set(entry.id, entry);
	};
	const link = (from: string, to: string) =>
		requireValue(pins.get(from)).connected_to.push(to);
	add(graph.exec("event", { start: true, execIn: false }));
	let previous = "event:exec-out-0";
	for (let index = 0; index < count; index++) {
		const id = (name: string) => `c${index}-${name}`;
		const at = (x: number, y: number) => [300 + index * 1400 + x, y];
		add(graph.exec(id("entry"), { dataOuts: 2, coordinates: at(0, 0) }));
		add(
			graph.exec(id("branch"), {
				execOuts: 2,
				dataIns: 1,
				dataOuts: 1,
				coordinates: at(230, 0),
			}),
		);
		for (const [name, y] of [
			["left", -90],
			["right", 110],
		] as const)
			add(
				graph.exec(id(name), {
					dataIns: 2,
					dataOuts: 1,
					coordinates: at(460, y),
				}),
			);
		add(
			graph.exec(id("merge"), {
				dataIns: 2,
				dataOuts: 1,
				coordinates: at(690, 0),
			}),
		);
		add(graph.pure(id("helper"), { coordinates: at(230, 200) }));
		add(graph.exec(id("exit"), { dataIns: 1, coordinates: at(920, 0) }));
		link(previous, `${id("entry")}:exec-in`);
		link(`${id("entry")}:exec-out-0`, `${id("branch")}:exec-in`);
		link(`${id("branch")}:exec-out-0`, `${id("left")}:exec-in`);
		link(`${id("branch")}:exec-out-1`, `${id("right")}:exec-in`);
		link(`${id("left")}:exec-out-0`, `${id("merge")}:exec-in`);
		link(`${id("right")}:exec-out-0`, `${id("merge")}:exec-in`);
		link(`${id("merge")}:exec-out-0`, `${id("exit")}:exec-in`);
		link(`${id("entry")}:out-0`, `${id("branch")}:in-0`);
		link(`${id("entry")}:out-1`, `${id("left")}:in-1`);
		link(`${id("branch")}:out-0`, `${id("left")}:in-0`);
		link(`${id("branch")}:out-0`, `${id("right")}:in-0`);
		link(`${id("entry")}:out-0`, `${id("helper")}:in-0`);
		link(`${id("helper")}:out-0`, `${id("right")}:in-1`);
		link(`${id("left")}:out-0`, `${id("merge")}:in-0`);
		link(`${id("right")}:out-0`, `${id("merge")}:in-1`);
		link(`${id("merge")}:out-0`, `${id("exit")}:in-0`);
		previous = `${id("exit")}:exec-out-0`;
	}
	return boardOf(graph);
}

function jaccard(a: readonly string[], b: readonly string[]): number {
	const shared = a.filter((id) => b.includes(id)).length;
	return shared / (a.length + b.length - shared);
}

function requireValue<T>(value: T | undefined): T {
	if (value === undefined) throw new Error("Missing grouping test value");
	return value;
}

function removeNodes(board: IBoard, ...ids: string[]): void {
	board.nodes = Object.fromEntries(
		Object.entries(board.nodes).filter(([id]) => !ids.includes(id)),
	);
}

function shiftNodes(board: IBoard, dx: number, dy: number): void {
	for (const node of Object.values(board.nodes)) {
		node.coordinates = [
			(node.coordinates?.[0] ?? 0) + dx,
			(node.coordinates?.[1] ?? 0) + dy,
		];
	}
}

function layer(id: string, node: INode, parent?: string): ILayer {
	return {
		id,
		name: id,
		type: ILayerType.Collapsed,
		parent_id: parent,
		coordinates: node.coordinates ?? [0, 0],
		pins: node.pins,
		nodes: {},
		comments: {},
		variables: {},
	};
}

describe("group suggestions", () => {
	test("finds a diamond with one entry and exit while keeping the event and sink visible", () => {
		const { board, members } = diamond();
		const suggestions = findGroupSuggestions({ board });
		expect(suggestions).toHaveLength(1);
		expect(suggestions[0].memberIds).toEqual(members);
		expect(suggestions[0].nodeCount).toBe(4);
		expect(suggestions[0].internalEdgeCount).toBe(4);
		expect(suggestions[0].inputCount).toBe(1);
		expect(suggestions[0].outputCount).toBe(1);
		expect(suggestions[0].boundaryPorts[0].connections).toEqual([
			{ nodeId: "event", pinId: "event:exec-out-0" },
		]);
		expect(suggestions[0].boundaryPorts[1].connections).toEqual([
			{ nodeId: "sink", pinId: "sink:exec-in" },
		]);
	});

	test("does not suggest ordinary chains, loose noise, or entire components", () => {
		const graph = new GraphBuilder();
		for (let index = 0; index < 16; index++) {
			graph.pure(`chain-${index}`);
			if (index) graph.dataLink(`chain-${index - 1}`, `chain-${index}`);
			graph.pure(`isolated-${index}`);
		}
		expect(findGroupSuggestions({ board: boardOf(graph) })).toEqual([]);
		const { board } = diamond();
		removeNodes(board, "event", "sink");
		expect(findGroupSuggestions({ board })).toEqual([]);
	});

	test("coalesces boundary pins using the runtime bridge keys", () => {
		const graph = new GraphBuilder();
		graph.exec("outside", { dataOuts: 1 });
		graph.exec("first", { dataIns: 1, dataOuts: 1 });
		graph.exec("second", { dataIns: 2, dataOuts: 1 });
		graph.pure("consumer-a");
		graph.pure("consumer-b");
		graph.execLink("outside", "first");
		graph.execLink("outside", "second");
		graph.dataLink("outside", "first");
		graph.dataLink("outside", "second");
		graph.dataLink("first", "second", 0, 1);
		graph.dataLink("second", "consumer-a");
		graph.dataLink("second", "consumer-b");
		const result = requireValue(
			evaluateGroupCandidate({ board: boardOf(graph) }, ["first", "second"]),
		);
		expect(result.inputCount).toBe(3);
		expect(result.outputCount).toBe(1);
		expect(result.boundaryPorts.map((port) => port.id)).toEqual([
			"input:first:exec-in",
			"input:outside:out-0",
			"input:second:exec-in",
			"output:second:out-0",
		]);
		expect(
			result.boundaryPorts.find((port) => port.id === "input:outside:out-0")
				?.connections,
		).toEqual([{ nodeId: "outside", pinId: "outside:out-0" }]);
		expect(
			result.boundaryPorts.find((port) => port.direction === "output")
				?.connections,
		).toEqual([
			{ nodeId: "consumer-a", pinId: "consumer-a:in-0" },
			{ nodeId: "consumer-b", pinId: "consumer-b:in-0" },
		]);
	});

	test("evaluates a safe pair below suggestion thresholds for apply revalidation", () => {
		const graph = new GraphBuilder();
		graph.pure("a");
		graph.pure("b");
		graph.dataLink("a", "b");
		const board = boardOf(graph);
		expect(evaluateGroupCandidate({ board }, ["a", "b"])?.nodeCount).toBe(2);
		expect(findGroupSuggestions({ board })).toEqual([]);
		expect(evaluateGroupCandidate({ board }, ["a", "missing"])).toBeUndefined();
	});

	test("selection restricts every proposed member", () => {
		const { board, members } = diamond();
		expect(
			findGroupSuggestions({ board, only: new Set(members) })[0]?.memberIds,
		).toEqual(members);
		expect(
			findGroupSuggestions({ board, only: new Set(["prepare", "left"]) }),
		).toEqual([]);
		expect(
			evaluateGroupCandidate(
				{ board, only: new Set(["prepare", "left"]) },
				members,
			),
		).toBeUndefined();
	});

	test("treats existing sibling layers as atomic members", () => {
		const { board, members } = diamond();
		board.layers.left = layer("left", board.nodes.left);
		removeNodes(board, "left");
		const insideGraph = new GraphBuilder();
		const hidden = insideGraph.pure("hidden");
		hidden.layer = "left";
		board.nodes.hidden = hidden;
		expect(findGroupSuggestions({ board })[0]?.memberIds).toEqual(members);
		expect(findGroupSuggestions({ board })[0]?.nodeCount).toBe(4);
		expect(
			evaluateGroupCandidate({ board }, [...members, "hidden"]),
		).toBeUndefined();
	});

	test("uses current-layer boundary nodes as outside preview endpoints", () => {
		const { board, members } = diamond();
		for (const node of Object.values(board.nodes)) node.layer = "container";
		const boundary = layer("container", board.nodes.event);
		boundary.pins = {};
		const inputPin = structuredClone(
			board.nodes.event.pins["event:exec-out-0"],
		);
		inputPin.pin_type = "Input" as typeof inputPin.pin_type;
		const outputPin = structuredClone(board.nodes.sink.pins["sink:exec-in"]);
		outputPin.pin_type = "Output" as typeof outputPin.pin_type;
		boundary.pins[inputPin.id] = inputPin;
		boundary.pins[outputPin.id] = outputPin;
		board.layers.container = boundary;
		removeNodes(board, "event", "sink");
		expect(findGroupSuggestions({ board })).toEqual([]);
		const evaluated = requireValue(
			evaluateGroupCandidate({ board, currentLayer: "container" }, members),
		);
		expect(evaluated.boundaryPorts[0].connections[0].nodeId).toBe(
			"container-input",
		);
		expect(evaluated.boundaryPorts[1].connections[0].nodeId).toBe(
			"container-return",
		);
	});

	test("includes internal reroute chains without inflating complexity", () => {
		const { graph, board, members } = diamond();
		const reroute = graph.pure("wire-dot");
		reroute.name = "reroute";
		reroute.coordinates = [645, 28];
		board.nodes[reroute.id] = reroute;
		board.nodes.left.pins["left:exec-out-0"].connected_to = ["wire-dot:in-0"];
		reroute.pins["wire-dot:out-0"].connected_to = ["combine:exec-in"];
		const suggestion = requireValue(findGroupSuggestions({ board })[0]);
		expect(suggestion.memberIds).toEqual([...members, "wire-dot"]);
		expect(suggestion.nodeCount).toBe(4);
		expect(suggestion.internalEdgeCount).toBe(4);
	});

	test("refuses function-reference cuts in either direction, including off-layer callers", () => {
		const { board, members } = diamond();
		const refs = (target: string) => ({
			can_reference_fns: true,
			can_be_referenced_by_fns: false,
			fn_refs: [target],
		});
		board.nodes.left.fn_refs = refs("right");
		expect(evaluateGroupCandidate({ board }, members)).toBeDefined();
		board.nodes.left.fn_refs = refs("sink");
		expect(evaluateGroupCandidate({ board }, members)).toBeUndefined();
		board.nodes.left.fn_refs = refs("off-layer");
		const graph = new GraphBuilder();
		const remote = graph.pure("off-layer");
		remote.layer = "elsewhere";
		board.nodes[remote.id] = remote;
		expect(evaluateGroupCandidate({ board }, members)).toBeUndefined();
		board.nodes.left.fn_refs = null;
		remote.fn_refs = refs("left");
		expect(evaluateGroupCandidate({ board }, members)).toBeUndefined();
	});

	test("examines function references hidden inside an atomic layer", () => {
		const { board, members } = diamond();
		board.layers.left = layer("left", board.nodes.left);
		removeNodes(board, "left");
		const graph = new GraphBuilder();
		const hidden = graph.pure("hidden");
		hidden.layer = "left";
		hidden.fn_refs = {
			can_reference_fns: true,
			can_be_referenced_by_fns: false,
			fn_refs: ["sink"],
		};
		board.nodes.hidden = hidden;
		expect(evaluateGroupCandidate({ board }, members)).toBeUndefined();
		hidden.fn_refs.fn_refs = ["right"];
		expect(evaluateGroupCandidate({ board }, members)).toBeDefined();
	});

	test("rejects splitting a directed cycle", () => {
		const { board, members } = diamond();
		board.nodes.combine.pins["combine:exec-out-0"].connected_to.push(
			"left:exec-in",
		);
		expect(
			evaluateGroupCandidate({ board }, ["prepare", "left"]),
		).toBeUndefined();
		expect(evaluateGroupCandidate({ board }, members)).toBeDefined();
		for (const suggestion of findGroupSuggestions({ board })) {
			expect(suggestion.memberIds.includes("left")).toBe(
				suggestion.memberIds.includes("combine"),
			);
		}
	});

	test("keeps topology IDs stable across moves and changes them when a boundary endpoint changes", () => {
		const { board, members } = diamond();
		const first = requireValue(evaluateGroupCandidate({ board }, members));
		board.nodes.left.coordinates = [2500, 1800];
		const moved = requireValue(evaluateGroupCandidate({ board }, members));
		expect(moved.id).toBe(first.id);
		expect(moved.bounds).not.toEqual(first.bounds);
		board.nodes.combine.pins["combine:exec-out-0"].connected_to = [
			"event:exec-out-0",
		];
		expect(evaluateGroupCandidate({ board }, members)?.id).not.toBe(first.id);
	});

	test("rejects event starts and disconnected membership", () => {
		const { board, members } = diamond();
		expect(
			evaluateGroupCandidate({ board }, ["event", ...members]),
		).toBeUndefined();
		expect(
			evaluateGroupCandidate({ board }, ["left", "right"]),
		).toBeUndefined();
		board.nodes.left.event_callback = true;
		expect(evaluateGroupCandidate({ board }, members)).toBeUndefined();
	});

	test("uses measured sizes for preview bounds", () => {
		const { board, members } = diamond();
		const result = requireValue(
			evaluateGroupCandidate(
				{ board, nodeSizes: new Map([["combine", [350, 90]]]) },
				members,
			),
		);
		expect(result.bounds).toEqual({ x: 230, y: 0, width: 810, height: 143 });
	});

	test("suggestions are stable under node, pin, and connection ordering", () => {
		const { board } = diamond();
		const expected = findGroupSuggestions({ board });
		const shuffled = structuredClone(board);
		shuffled.nodes = Object.fromEntries(
			Object.entries(shuffled.nodes).reverse(),
		);
		for (const node of Object.values(shuffled.nodes)) {
			node.pins = Object.fromEntries(Object.entries(node.pins).reverse());
			for (const pin of Object.values(node.pins)) pin.connected_to.reverse();
		}
		expect(findGroupSuggestions({ board: shuffled })).toEqual(expected);
	});

	test("returns at most three disjoint regions", () => {
		const board = { nodes: {}, layers: {}, comments: {} } as IBoard;
		for (let index = 0; index < 5; index++) {
			const cluster = diamond(`cluster-${index}-`).board;
			shiftNodes(cluster, 0, index * 300);
			Object.assign(board.nodes, cluster.nodes);
		}
		const suggestions = findGroupSuggestions({ board });
		expect(suggestions).toHaveLength(3);
		const ids = suggestions.flatMap((suggestion) => suggestion.memberIds);
		expect(new Set(ids).size).toBe(ids.length);
	});

	test("skipped memberships make room for the next-ranked region", () => {
		const board = { nodes: {}, layers: {}, comments: {} } as IBoard;
		for (let index = 0; index < 4; index++) {
			const cluster = diamond(`cluster-${index}-`).board;
			shiftNodes(cluster, 0, index * 300);
			Object.assign(board.nodes, cluster.nodes);
		}
		const first = findGroupSuggestions({ board });
		expect(first).toHaveLength(3);
		const skipped = first[0];
		const next = findGroupSuggestions({
			board,
			skip: [[...skipped.memberIds].reverse()],
		});
		expect(next).toHaveLength(3);
		expect(next.map((suggestion) => suggestion.id)).not.toContain(skipped.id);
		expect(next.map((suggestion) => suggestion.id)).toEqual(
			expect.arrayContaining([first[1].id, first[2].id]),
		);
	});

	test("never re-offers a near-identical variant of a skipped group", () => {
		const board = chainedClusters(12);
		const skip: string[][] = [];
		let offered = 0;
		for (let round = 0; round < 4; round++) {
			const found = findGroupSuggestions({ board, skip });
			for (const suggestion of found) {
				for (const skipped of skip)
					expect(jaccard(skipped, suggestion.memberIds)).toBeLessThan(0.5);
				skip.push(suggestion.memberIds);
			}
			offered += found.length;
		}
		expect(offered).toBeGreaterThan(3);
	});

	test("searches and ranks the visible region first on a board too large to search whole", () => {
		const board = chainedClusters(90);
		const farX = requireValue(board.nodes["c80-branch"].coordinates?.[0]);
		const focus = { x: farX - 400, y: -600, width: 2400, height: 1200 };
		const visible = findGroupSuggestions({ board, focus });
		expect(visible.length).toBeGreaterThan(0);
		const first = visible[0];
		expect(
			first.bounds.x < focus.x + focus.width &&
				first.bounds.x + first.bounds.width > focus.x,
		).toBe(true);
		const clusterOf = (id: string) => Number(id.split("-")[0].slice(1));
		for (const suggestion of visible)
			for (const id of suggestion.memberIds)
				expect(Math.abs(clusterOf(id) - 80)).toBeLessThan(25);
		const unfocused = findGroupSuggestions({ board });
		expect(
			unfocused.some((suggestion) =>
				suggestion.memberIds.some((id) => clusterOf(id) >= 65),
			),
		).toBe(false);
	});

	test("revalidates several memberships against one graph, keeping their order", () => {
		const { board, members } = diamond();
		const lists = [members, ["prepare", "sink"], [...members].reverse()];
		const batch = evaluateGroupCandidates({ board }, lists);
		expect(batch).toEqual(
			lists.map((memberIds) => evaluateGroupCandidate({ board }, memberIds)),
		);
		expect(batch[0]?.memberIds).toEqual(members);
		expect(batch[1]).toBeUndefined();
		expect(evaluateGroupCandidates({ board }, [])).toEqual([]);
	});

	test("keeps one search on about 5,000 nodes interactive", () => {
		const board = chainedClusters(700);
		const start = performance.now();
		const suggestions = findGroupSuggestions({ board });
		expect(performance.now() - start).toBeLessThan(750);
		expect(suggestions).toHaveLength(3);
	});

	test("hides the lower-ranked ghost when disjoint groups occupy interleaved screen regions", () => {
		const first = diamond("a-");
		const interleaved = diamond("b-");
		interleaved.board.nodes["b-prepare"].coordinates = [230, 60];
		interleaved.board.nodes["b-left"].coordinates = [460, 180];
		interleaved.board.nodes["b-right"].coordinates = [460, 280];
		interleaved.board.nodes["b-combine"].coordinates = [690, 60];
		const separate = diamond("c-");
		shiftNodes(separate.board, 0, 500);
		const board = {
			...first.board,
			nodes: {
				...first.board.nodes,
				...interleaved.board.nodes,
				...separate.board.nodes,
			},
		};
		const firstCandidate = requireValue(
			evaluateGroupCandidate({ board }, first.members),
		);
		const interleavedCandidate = requireValue(
			evaluateGroupCandidate({ board }, interleaved.members),
		);
		expect(firstCandidate.score).toBeGreaterThan(interleavedCandidate.score);
		expect(
			firstCandidate.memberIds.every(
				(id) => !interleavedCandidate.memberIds.includes(id),
			),
		).toBe(true);
		expect(
			firstCandidate.bounds.y + firstCandidate.bounds.height,
		).toBeGreaterThan(interleavedCandidate.bounds.y);
		const suggestions = findGroupSuggestions({ board });
		expect(suggestions).toHaveLength(2);
		expect(
			suggestions.map((suggestion) => suggestion.memberIds),
		).toContainEqual(first.members);
		expect(
			suggestions.map((suggestion) => suggestion.memberIds),
		).toContainEqual(separate.members);
	});

	test("prefers compact hotspots over joining distant clusters through a long link", () => {
		const first = diamond("a-");
		const second = diamond("b-");
		for (const node of Object.values(second.board.nodes))
			node.coordinates = [
				(node.coordinates?.[0] ?? 0) + 10000,
				node.coordinates?.[1] ?? 0,
			];
		const board = {
			...first.board,
			nodes: { ...first.board.nodes, ...second.board.nodes },
		};
		board.nodes["a-combine"].pins["a-combine:exec-out-0"].connected_to = [
			"b-prepare:exec-in",
		];
		removeNodes(board, "a-sink", "b-event");
		const suggestions = findGroupSuggestions({ board });
		expect(suggestions).toHaveLength(2);
		expect(
			suggestions.every((suggestion) => suggestion.bounds.width < 2000),
		).toBe(true);
		expect(
			suggestions.some((suggestion) =>
				suggestion.memberIds.every((id) => id.startsWith("a-")),
			),
		).toBe(true);
		expect(
			suggestions.some((suggestion) =>
				suggestion.memberIds.every((id) => id.startsWith("b-")),
			),
		).toBe(true);
	});

	test("bounds work on about 500 nodes", () => {
		const board = { nodes: {}, layers: {}, comments: {} } as IBoard;
		for (let index = 0; index < 83; index++) {
			const cluster = diamond(`cluster-${index}-`).board;
			shiftNodes(cluster, 0, index * 300);
			Object.assign(board.nodes, cluster.nodes);
		}
		const start = performance.now();
		const suggestions = findGroupSuggestions({ board });
		expect(performance.now() - start).toBeLessThan(1500);
		expect(suggestions).toHaveLength(3);
	});
});

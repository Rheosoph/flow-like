import { describe, expect, test } from "bun:test";
import { ILayerType } from "../../../lib/schema/flow/board";
import { IPinType, IVariableType } from "../../../lib/schema/flow/pin";
import {
	buildBoardGraph,
	layerPath,
	missedUpstream,
	nodeLabel,
	upstreamNodes,
} from "./board-graph";
import { makeBoard, makeNode, makePin } from "./test-fixtures";

const { Input, Output } = IPinType;

/**
 * The brief's sample: For Each triggers Set Field and Upsert; Set Field's
 * struct and Open Database's connection feed Upsert; a Path node feeds Open
 * Database. Open Database never ran.
 */
function sampleBoard() {
	const each = makeNode("each", "For Each", [
		makePin("each_exec_out", Output, {
			exec: true,
			connectedTo: ["setf_exec_in"],
		}),
		makePin("each_value", Output, { connectedTo: ["setf_in"] }),
		makePin("each_done", Output, {
			exec: true,
			connectedTo: ["upsert_exec_in"],
		}),
	]);
	const setf = makeNode("setf", "Set Field", [
		makePin("setf_exec_in", Input, {
			exec: true,
			dependsOn: ["each_exec_out"],
		}),
		makePin("setf_in", Input, { dependsOn: ["each_value"] }),
		makePin("setf_out", Output, { connectedTo: ["upsert_value"] }),
	]);
	const path = makeNode("path", "Make Path", [
		makePin("path_out", Output, { connectedTo: ["open_path"] }),
	]);
	const open = makeNode("open", "Open Database", [
		makePin("open_path", Input, { dependsOn: ["path_out"] }),
		makePin("open_db", Output, { connectedTo: ["upsert_db"] }),
	]);
	const upsert = makeNode("upsert", "Upsert", [
		makePin("upsert_exec_in", Input, { exec: true, dependsOn: ["each_done"] }),
		makePin("upsert_db", Input, {
			name: "Database",
			dependsOn: ["open_db"],
			index: 1,
		}),
		makePin("upsert_value", Input, {
			name: "Value",
			dependsOn: ["setf_out"],
			index: 2,
		}),
	]);
	return makeBoard([each, setf, path, open, upsert]);
}

describe("upstreamNodes", () => {
	test("data sources transitively, execution one hop", () => {
		const graph = buildBoardGraph(sampleBoard());
		expect(upstreamNodes(graph, "upsert").sort()).toEqual(
			["each", "open", "path", "setf"].sort(),
		);
		expect(upstreamNodes(graph, "setf").sort()).toEqual(["each"]);
		expect(upstreamNodes(graph, "path")).toEqual([]);
		expect(upstreamNodes(graph, "missing")).toEqual([]);
	});

	test("reads edges stored on only one side", () => {
		const a = makeNode("a", "A", [
			makePin("a_out", Output, { connectedTo: ["b_in"] }),
		]);
		const b = makeNode("b", "B", [makePin("b_in", Input)]);
		const c = makeNode("c", "C", [makePin("c_out", Output)]);
		const d = makeNode("d", "D", [
			makePin("d_in", Input, { dependsOn: ["c_out"] }),
		]);
		const graph = buildBoardGraph(makeBoard([a, b, c, d]));
		expect(upstreamNodes(graph, "b")).toEqual(["a"]);
		expect(upstreamNodes(graph, "d")).toEqual(["c"]);
	});

	test("looks through collapsed layer pins but stops at function boundaries", () => {
		const outer = makeNode("outer", "Outer", [
			makePin("outer_out", Output, { connectedTo: ["bridge"] }),
		]);
		const inner = makeNode(
			"inner",
			"Inner",
			[makePin("inner_in", Input, { dependsOn: ["bridge"] })],
			"layer",
		);
		const fnInner = makeNode(
			"fn_inner",
			"Fn Inner",
			[makePin("fn_in", Input, { dependsOn: ["fn_param"] })],
			"fn",
		);
		const layers = {
			layer: {
				id: "layer",
				name: "Serialize Routes",
				type: ILayerType.Collapsed,
				pins: {
					bridge: makePin("bridge", Input, {
						dependsOn: ["outer_out"],
						connectedTo: ["inner_in"],
					}),
				},
			},
			fn: {
				id: "fn",
				name: "Fn",
				type: ILayerType.Function,
				pins: {
					fn_param: makePin("fn_param", Input, { dependsOn: ["outer_out"] }),
				},
			},
		};
		const graph = buildBoardGraph(
			makeBoard([outer, inner, fnInner], { layers: layers as never }),
		);
		expect(upstreamNodes(graph, "inner")).toEqual(["outer"]);
		expect(upstreamNodes(graph, "fn_inner")).toEqual([]);
	});

	test("survives cycles", () => {
		const a = makeNode("a", "A", [
			makePin("a_in", Input, { dependsOn: ["b_out"] }),
			makePin("a_out", Output, { connectedTo: ["b_in"] }),
		]);
		const b = makeNode("b", "B", [
			makePin("b_in", Input, { dependsOn: ["a_out"] }),
			makePin("b_out", Output, { connectedTo: ["a_in"] }),
		]);
		expect(upstreamNodes(buildBoardGraph(makeBoard([a, b])), "a")).toEqual([
			"b",
		]);
	});
});

describe("missedUpstream", () => {
	function reroutedLookup() {
		const source = makeNode("entry", "Found Summary", [
			makePin("path", Output, { connectedTo: ["route_a_in"] }),
		]);
		const first = makeNode("route_a", "Reroute", [
			makePin("route_a_in", Input, { dependsOn: ["path"] }),
			makePin("route_a_out", Output, { connectedTo: ["route_b_in"] }),
		]);
		const second = makeNode("route_b", "Reroute", [
			makePin("route_b_in", Input, { dependsOn: ["route_a_out"] }),
			makePin("route_b_out", Output, { connectedTo: ["key"] }),
		]);
		const lookup = makeNode("lookup", "Get Value", [
			makePin("key", Input, { name: "Key", dependsOn: ["route_b_out"] }),
			makePin("value", Output, { connectedTo: ["summary"] }),
		]);
		const setter = makeNode("setter", "Set Summary", [
			makePin("summary", Input, { dependsOn: ["value"] }),
		]);
		return makeBoard([source, first, second, lookup, setter]);
	}

	test("does not report compiler-spliced reroutes as missed executions", () => {
		const graph = buildBoardGraph(reroutedLookup());
		expect(
			missedUpstream(graph, "setter", new Set(["entry", "lookup", "setter"])),
		).toEqual([]);
		// Keep reroutes available when explicitly scoping the logs to upstream nodes.
		expect(upstreamNodes(graph, "setter").sort()).toEqual([
			"entry",
			"lookup",
			"route_a",
			"route_b",
		]);
	});

	test("reports the source behind reroutes with its consuming pin", () => {
		const graph = buildBoardGraph(reroutedLookup());
		expect(
			missedUpstream(graph, "setter", new Set(["lookup", "setter"])),
		).toEqual([{ nodeId: "entry", feeds: { nodeId: "lookup", pin: "Key" } }]);
	});

	test("looks through reroutes across collapsed layers and stops at function parameters", () => {
		const board = reroutedLookup();
		board.nodes.route_a.pins.route_a_out.connected_to = ["bridge"];
		board.nodes.route_b.pins.route_b_in.depends_on = ["bridge"];
		board.layers = {
			layer: {
				id: "layer",
				name: "Layer",
				type: ILayerType.Collapsed,
				pins: {
					bridge: makePin("bridge", Input, {
						dependsOn: ["route_a_out"],
						connectedTo: ["route_b_in"],
					}),
				},
			},
		} as never;
		expect(
			missedUpstream(buildBoardGraph(board), "setter", new Set(["lookup"])),
		).toEqual([{ nodeId: "entry", feeds: { nodeId: "lookup", pin: "Key" } }]);
		board.layers.layer.type = ILayerType.Function;
		expect(
			missedUpstream(buildBoardGraph(board), "setter", new Set(["lookup"])),
		).toEqual([]);
	});

	test.each(["literal", "wasm", "geometry", "extra-pin"])(
		"still reports a retained %s reroute that did not run",
		(kind) => {
			const board = reroutedLookup();
			const reroute = board.nodes.route_b;
			if (kind === "literal") {
				reroute.pins.route_b_in.depends_on = [];
				reroute.pins.route_b_in.default_value = [34, 107, 101, 121, 34];
				board.nodes.route_a.pins.route_a_out.connected_to = [];
			} else if (kind === "wasm") {
				reroute.wasm = { package_id: "custom-reroute", permissions: [] };
			} else if (kind === "geometry") {
				reroute.pins.route_b_out.data_type = IVariableType.Geometry;
			} else {
				reroute.pins.extra = makePin("extra", Input);
			}
			expect(
				missedUpstream(buildBoardGraph(board), "setter", new Set(["lookup"])),
			).toEqual([
				{ nodeId: "route_b", feeds: { nodeId: "lookup", pin: "Key" } },
			]);
		},
	);

	test("terminates when a reroute chain contains a cycle", () => {
		const board = reroutedLookup();
		board.nodes.entry.pins.path.connected_to = [];
		board.nodes.route_a.pins.route_a_in.depends_on = ["route_b_out"];
		board.nodes.route_b.pins.route_b_out.connected_to.push("route_a_in");
		expect(
			missedUpstream(buildBoardGraph(board), "setter", new Set(["lookup"])),
		).toEqual([]);
	});

	test("names the nearest data source that never ran and what it feeds", () => {
		const graph = buildBoardGraph(sampleBoard());
		const visited = new Set(["each", "setf", "upsert"]);
		expect(missedUpstream(graph, "upsert", visited)).toEqual([
			{ nodeId: "open", feeds: { nodeId: "upsert", pin: "Database" } },
		]);
	});

	test("walks through sources that ran", () => {
		const graph = buildBoardGraph(sampleBoard());
		const visited = new Set(["each", "setf", "upsert", "open"]);
		expect(missedUpstream(graph, "upsert", visited)).toEqual([
			{ nodeId: "path", feeds: { nodeId: "open", pin: "open_path" } },
		]);
		expect(
			missedUpstream(
				graph,
				"upsert",
				new Set(["each", "setf", "upsert", "open", "path"]),
			),
		).toEqual([]);
	});
});

describe("labels", () => {
	test("node label prefers the friendly name", () => {
		const board = sampleBoard();
		expect(nodeLabel(board.nodes.upsert)).toBe("Upsert");
		expect(nodeLabel({ ...board.nodes.upsert, friendly_name: " " })).toBe(
			"upsert",
		);
		expect(nodeLabel(undefined)).toBeUndefined();
	});

	test("layer path runs from the board through nested layers", () => {
		const node = makeNode("n", "N", [], "inner");
		const board = makeBoard([node], {
			layers: {
				outer: {
					id: "outer",
					name: "Retrieve Entities In Area",
					parent_id: null,
				},
				inner: { id: "inner", name: "Serialize Routes", parent_id: "outer" },
			} as never,
		});
		expect(layerPath(board, "n")).toEqual([
			"main.flow",
			"Retrieve Entities In Area",
			"Serialize Routes",
		]);
		expect(layerPath(board, "missing")).toEqual(["main.flow"]);
	});
});

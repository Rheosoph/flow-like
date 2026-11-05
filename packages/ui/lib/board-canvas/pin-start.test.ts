import { expect, test } from "bun:test";
import { isValidConnection } from "../flow-board-utils";
import type { ILayer } from "../schema/flow/board";
import { ICommandType } from "../schema/flow/board/commands/generic-command";
import type { INode } from "../schema/flow/node";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../schema/flow/pin";
import { buildConnectionCommands } from "./connection-commands";
import { BoardHitIndex, ConnectionGesture } from "./interaction";
import { resolvePinStart } from "./pin-start";
import type { Scene } from "./scene";

const scene: Scene = {
	nodes: [
		{
			id: "source",
			x: 0,
			y: 0,
			width: 150,
			height: 80,
			label: "Source",
			kind: "standard",
			selected: false,
			dimmed: false,
			pins: [
				{
					id: "value",
					direction: "output",
					x: 150,
					y: 28,
					label: "Value",
					color: [255, 255, 255, 255],
					kind: "data",
				},
				{
					id: "next",
					direction: "output",
					x: 150,
					y: 43,
					label: "Next",
					color: [255, 255, 255, 255],
					kind: "data",
				},
				{
					id: "ref_out_source",
					direction: "output",
					x: 75,
					y: 80,
					label: "",
					color: [255, 255, 255, 255],
					kind: "reference",
				},
			],
		},
	],
	edges: [
		{
			id: "edge",
			source: { nodeId: "source", pinId: "value" },
			target: { nodeId: "other", pinId: "input" },
			kind: "data",
			selected: true,
			color: [255, 255, 255, 255],
		},
	],
};
const index = new BoardHitIndex(scene);
const canConnect = () => true;

test("explicit DOM handles win over closer pins and selected wire endpoints", () => {
	const candidate = resolvePinStart(index, { x: 150, y: 28 }, 1, {
		canConnect,
		explicit: { nodeId: "source", pinId: "next" },
	});
	expect(candidate?.pin.id).toBe("next");
	expect(candidate?.reconnect).toBeUndefined();
});

test("a high-zoom function-reference handle corner activates its exact handle", () => {
	const candidate = resolvePinStart(index, { x: 80.5, y: 85.5 }, 2.5, {
		canConnect,
		explicit: { nodeId: "source", pinId: "ref_out_source" },
	});
	expect(candidate?.pin.id).toBe("ref_out_source");
});

test("an explicit handle starts a new wire even when its existing wire is selected", () => {
	const candidate = resolvePinStart(index, { x: 155, y: 33 }, 3, {
		canConnect,
		explicit: { nodeId: "source", pinId: "value" },
	});
	expect(candidate?.pin.id).toBe("value");
	expect(candidate?.reconnect).toBeUndefined();
});

test("geometric reconnect affordances remain available outside the visible pin", () => {
	const candidate = resolvePinStart(index, { x: 155, y: 28 }, 1, {
		canConnect,
	});
	expect(candidate?.pin.id).toBe("value");
	expect(candidate?.reconnect?.edge.id).toBe("edge");
	expect(candidate?.reconnect?.end).toBe("source");
});

test("hover and press use the same 12 CSS pixel snapping boundary", () => {
	expect(
		resolvePinStart(index, { x: 75 - 12 / 0.25, y: 80 }, 0.25, { canConnect })
			?.pin.id,
	).toBe("ref_out_source");
	expect(
		resolvePinStart(index, { x: 75, y: 80 + 12.1 / 0.25 }, 0.25, {
			canConnect,
		}),
	).toBeUndefined();
});

test("nested editors, locked nodes, and foreground node ownership cannot advertise a connection", () => {
	const point = { x: 150, y: 28 };
	const explicit = { nodeId: "source", pinId: "value" };
	expect(
		resolvePinStart(index, point, 1, { canConnect, explicit, blocked: true }),
	).toBeUndefined();
	expect(
		resolvePinStart(index, point, 1, { canConnect: () => false, explicit }),
	).toBeUndefined();
	expect(
		resolvePinStart(index, point, 1, { canConnect, nodeId: "foreground" }),
	).toBeUndefined();
});

test("a stale explicit handle never falls through to a different nearby pin", () => {
	expect(
		resolvePinStart(index, { x: 150, y: 28 }, 1, {
			canConnect,
			explicit: { nodeId: "source", pinId: "removed" },
		}),
	).toBeUndefined();
});

function connectedScene(
	kind: "data" | "execution" | "reference" = "data",
): Scene {
	const source = structuredClone(scene.nodes[0]);
	source.pins = [{ ...source.pins[0], id: "out", kind }];
	const target = { ...structuredClone(source), id: "target", x: 500 };
	target.pins = [{ ...target.pins[0], id: "in", direction: "input", x: 0 }];
	if (kind === "reference") {
		source.pins[0].x = 75;
		source.pins[0].y = 80;
		target.x = 0;
		target.y = 400;
		target.pins[0].x = 75;
		target.pins[0].y = 0;
	}
	return {
		nodes: [source, target],
		edges: [
			{
				id: "wire",
				source: { nodeId: source.id, pinId: "out" },
				target: { nodeId: target.id, pinId: "in" },
				kind,
				selected: false,
				color: [255, 255, 255, 255],
			},
		],
	};
}

test("selected wires cannot reverse new typed connections between an input and a reroute", () => {
	const graph = connectedScene();
	graph.edges[0].selected = true;
	const reroute = {
		...structuredClone(graph.nodes[0]),
		id: "reroute",
		x: 250,
		y: 150,
		kind: "reroute" as const,
		width: 16,
		height: 12,
	};
	reroute.pins = [{ ...reroute.pins[0], id: "route_out", x: 15, y: 6 }];
	const other = { ...structuredClone(graph.nodes[1]), id: "other", y: 150 };
	other.pins[0].id = "other.in";
	graph.nodes.push(reroute, other);
	graph.edges.push({
		...graph.edges[0],
		id: "rerouted-wire",
		source: { nodeId: "reroute", pinId: "route_out" },
		target: { nodeId: "other", pinId: "other.in" },
	});
	const boardNodes: Record<string, INode> = {};
	const pinCache = new Map<string, [IPin, INode | ILayer, boolean]>();
	const refs = {
		path: '{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}',
	};
	for (const rendered of graph.nodes) {
		const pins = rendered.pins.map(
			(p): IPin => ({
				id: p.id,
				name: p.id,
				friendly_name: p.id,
				description: "",
				index: 1,
				pin_type: p.direction === "output" ? IPinType.Output : IPinType.Input,
				data_type: IVariableType.Struct,
				value_type: IValueType.Normal,
				schema: "path",
				options: { enforce_schema: true },
				connected_to: graph.edges
					.filter((edge) => edge.source.pinId === p.id)
					.map((edge) => edge.target.pinId),
				depends_on: graph.edges
					.filter((edge) => edge.target.pinId === p.id)
					.map((edge) => edge.source.pinId),
			}),
		);
		const node: INode = {
			id: rendered.id,
			name: rendered.id,
			friendly_name: rendered.id,
			description: "",
			category: "test",
			pins: Object.fromEntries(pins.map((p) => [p.id, p])),
		};
		boardNodes[node.id] = node;
		for (const p of pins) pinCache.set(p.id, [p, node, true]);
	}
	const output = { nodeId: "reroute", pinId: "route_out" };
	const input = { nodeId: "target", pinId: "in" };
	for (const [outgoingSelected, explicit] of [
		[false, false],
		[false, true],
		[true, false],
		[true, true],
	]) {
		graph.edges[1].selected = outgoingSelected;
		const index = new BoardHitIndex(graph);
		for (const [from, to] of [
			[output, input],
			[input, output],
		]) {
			const fromPin = index.pin(from);
			const toPin = index.pin(to);
			if (!fromPin || !toPin) throw new Error("Missing fixture endpoint");
			const candidate = resolvePinStart(index, fromPin, 1, {
				canConnect,
				explicit: explicit ? from : undefined,
			});
			expect(candidate).toBeDefined();
			if (!candidate) throw new Error("Expected a connection start");
			const endpoint = candidate.reconnect;
			const gesture = endpoint
				? new ConnectionGesture(
						endpoint.edge[endpoint.end === "source" ? "target" : "source"],
						{ edgeId: endpoint.edge.id, end: endpoint.end },
					)
				: new ConnectionGesture({
						nodeId: candidate.pin.nodeId,
						pinId: candidate.pin.id,
					});
			const action = gesture.drop(index, toPin, 1, (connection) =>
				isValidConnection(connection, pinCache, refs),
			);
			expect(action.kind).toBe("connect");
			if (action.kind !== "connect")
				throw new Error("Expected a connection drop");
			expect(action.oldEdgeId).toBeUndefined();
			const commands = buildConnectionCommands({
				connection: action.connection,
				boardNodes,
				pinCache,
				refs,
			});
			expect(
				commands?.map((command) => [
					command.command_type,
					command.from_pin,
					command.to_pin,
				]),
			).toEqual([
				[ICommandType.DisconnectPin, "out", "in"],
				[ICommandType.ConnectPin, "route_out", "in"],
			]);
		}
	}
});

test("unselected wire corridors reconnect either end at every zoom, including function references", () => {
	for (const kind of ["data", "execution", "reference"] as const) {
		const graph = connectedScene(kind);
		const hit = new BoardHitIndex(graph);
		for (const zoom of [0.5, 1, 2]) {
			for (const end of ["source", "target"] as const) {
				const endpoint = graph.edges[0][end];
				const pin = hit.pin(endpoint);
				if (!pin) throw new Error("Missing fixture endpoint");
				const along = (end === "source" ? 18 : -18) / zoom;
				const point = {
					x: pin.x + (kind === "reference" ? 0 : along),
					y: pin.y + (kind === "reference" ? along : 0),
				};
				const hover = resolvePinStart(hit, point, zoom, { canConnect });
				const press = resolvePinStart(hit, point, zoom, { canConnect });
				expect(hover).toEqual(press);
				expect(press?.reconnect).toEqual({ edge: graph.edges[0], end });
			}
		}
	}
});

test("actual pins create new wires while nearby wire corridors beat fuzzy pin capture", () => {
	const graph = connectedScene();
	const hit = new BoardHitIndex(graph);
	const pin = hit.pin(graph.edges[0].source);
	if (!pin) throw new Error("Missing fixture endpoint");
	expect(
		resolvePinStart(hit, pin, 1, { canConnect })?.reconnect,
	).toBeUndefined();
	expect(
		resolvePinStart(hit, { x: pin.x + 3, y: pin.y }, 1, { canConnect })
			?.reconnect,
	).toBeUndefined();
	expect(
		resolvePinStart(hit, { x: pin.x + 8, y: pin.y }, 1, { canConnect })
			?.reconnect?.edge.id,
	).toBe("wire");
	expect(
		resolvePinStart(hit, { x: pin.x + 8, y: pin.y }, 1, {
			canConnect,
			explicit: graph.edges[0].source,
		})?.reconnect,
	).toBeUndefined();
	expect(
		resolvePinStart(hit, { x: pin.x, y: pin.y + 8 }, 1, { canConnect })
			?.reconnect,
	).toBeUndefined();
});

test("mid-curve, far endpoints and unrelated crossing wires do not reconnect", () => {
	const graph = connectedScene();
	const crossing = structuredClone(graph);
	crossing.nodes[0].id = "cross-source";
	crossing.nodes[0].x = -100;
	crossing.nodes[0].y = -160;
	crossing.nodes[1].id = "cross-target";
	crossing.nodes[1].x = 300;
	crossing.nodes[1].y = 160;
	crossing.edges[0].id = "cross-wire";
	crossing.edges[0].source.nodeId = "cross-source";
	crossing.edges[0].target.nodeId = "cross-target";
	graph.nodes.push(...crossing.nodes);
	graph.edges.push(...crossing.edges);
	const hit = new BoardHitIndex(graph);
	expect(
		resolvePinStart(hit, { x: 325, y: 28 }, 1, { canConnect }),
	).toBeUndefined();
	expect(
		resolvePinStart(hit, { x: 190, y: 28 }, 1, { canConnect }),
	).toBeUndefined();
	expect(
		resolvePinStart(hit, { x: 168, y: 28 }, 1, { canConnect })?.reconnect?.edge
			.id,
	).toBe("wire");
});

test("actual SVG path identity supports custom curves and excludes other nearby wires", () => {
	const graph = connectedScene();
	const hit = new BoardHitIndex(graph);
	const point = { x: 150, y: 48 };
	expect(resolvePinStart(hit, point, 1, { canConnect })).toBeUndefined();
	expect(
		resolvePinStart(hit, point, 1, { canConnect, explicitEdgeId: "wire" })
			?.reconnect?.end,
	).toBe("source");
	expect(
		resolvePinStart(hit, { x: 150, y: 68 }, 1, {
			canConnect,
			explicitEdgeId: "wire",
		}),
	).toBeUndefined();
	expect(
		resolvePinStart(hit, point, 1, { canConnect, explicitEdgeId: "stale" }),
	).toBeUndefined();
});

test("original edge permissions apply equally to selected endpoints and direct reconnection", () => {
	const graph = connectedScene();
	graph.edges[0].selected = true;
	const hit = new BoardHitIndex(graph);
	const canReconnect = (_edge: unknown, end: "source" | "target") =>
		end === "target";
	expect(
		resolvePinStart(hit, { x: 168, y: 28 }, 1, { canConnect, canReconnect }),
	).toBeUndefined();
	expect(
		resolvePinStart(hit, { x: 482, y: 28 }, 1, { canConnect, canReconnect })
			?.reconnect?.end,
	).toBe("target");
	expect(
		resolvePinStart(hit, { x: 150, y: 28 }, 1, {
			canConnect,
			canReconnect,
			explicit: graph.edges[0].source,
		})?.reconnect,
	).toBeUndefined();
});

test("reroute centers stay draggable below full zoom while exact pins and outside wires remain connectable", () => {
	const graph = connectedScene();
	const reroute = graph.nodes[0];
	reroute.kind = "reroute";
	reroute.width = 16;
	reroute.height = 12;
	reroute.pins[0].x = 16;
	reroute.pins[0].y = 6;
	reroute.pins.push({ ...reroute.pins[0], id: "in", direction: "input", x: 0 });
	graph.nodes[1].pins[0].y = 6;
	for (const selected of [false, true]) {
		graph.edges[0].selected = selected;
		const hit = new BoardHitIndex(graph);
		for (const zoom of [0.1, 0.25, 0.5, 1]) {
			expect(
				resolvePinStart(hit, { x: 8, y: 6 }, zoom, { canConnect }),
			).toBeUndefined();
			expect(
				resolvePinStart(hit, { x: 8, y: 6 }, zoom, {
					canConnect,
					explicit: graph.edges[0].source,
				}),
			).toBeUndefined();
			expect(
				resolvePinStart(hit, { x: 16, y: 6 }, zoom, { canConnect })?.pin.id,
			).toBe("out");
			expect(
				resolvePinStart(hit, { x: 16 + 18 / zoom, y: 6 }, zoom, { canConnect })
					?.reconnect?.end,
			).toBe("source");
		}
	}
});

test("overlapping endpoint corridors resolve deterministically and follow moved nodes", () => {
	const graph = connectedScene();
	graph.edges.push({ ...graph.edges[0], id: "other-wire" });
	const first = new BoardHitIndex(graph);
	const reversed = new BoardHitIndex({
		...graph,
		edges: [...graph.edges].reverse(),
	});
	const point = { x: 168, y: 28 };
	expect(
		resolvePinStart(first, point, 1, { canConnect })?.reconnect?.edge.id,
	).toBe("other-wire");
	expect(resolvePinStart(reversed, point, 1, { canConnect })).toEqual(
		resolvePinStart(first, point, 1, { canConnect }),
	);
	first.moveNodes([
		{ id: "source", x: 0, y: 200 },
		{ id: "target", x: 500, y: 200 },
	]);
	expect(resolvePinStart(first, point, 1, { canConnect })).toBeUndefined();
	expect(
		resolvePinStart(first, { x: 168, y: 228 }, 1, { canConnect })?.reconnect
			?.end,
	).toBe("source");
});

test("ordinary node bodies remain draggable at overview while real pins and external snapping stay available", () => {
	const graph = { nodes: structuredClone(scene.nodes), edges: [] };
	const hit = new BoardHitIndex(graph);
	for (const zoom of [0.02, 0.05, 0.25, 1]) {
		expect(
			resolvePinStart(hit, { x: 75, y: 40 }, zoom, { canConnect }),
		).toBeUndefined();
		expect(
			resolvePinStart(hit, { x: 150, y: 28 }, zoom, { canConnect })?.pin.id,
		).toBe("value");
		expect(
			resolvePinStart(hit, { x: 150 + 11 / zoom, y: 28 }, zoom, { canConnect })
				?.pin.id,
		).toBe("value");
		expect(
			resolvePinStart(hit, { x: 148, y: 33 }, zoom, {
				canConnect,
				explicit: { nodeId: "source", pinId: "value" },
			})?.pin.id,
		).toBe("value");
	}
});

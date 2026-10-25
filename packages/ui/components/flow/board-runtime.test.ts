import { expect, test } from "bun:test";
import { type Node, type NodeChange, Position } from "@xyflow/react";
import { getViewportForBounds } from "@xyflow/system";
import { type BoardRuntime, createBoardRuntime } from "./board-runtime";

function node(id: string, x = 0, y = 0): Node {
	return {
		id,
		position: { x, y },
		measured: { width: 150, height: 80 },
		data: {},
	};
}

test("native graph adopts geometry and preserves unchanged rows across edits", () => {
	const runtime = createBoardRuntime();
	const a = node("a", 10, 20);
	const b = node("b", 250, 100);
	const nodes = [a, b];
	const edge = { id: "e", source: "a", target: "b" };
	runtime.configure({ nodes, edges: [edge] });
	const before = runtime.store.getState();
	expect(runtime.flow.getNode("a")).toBe(a);
	expect(runtime.flow.getInternalNode("a")?.internals.positionAbsolute).toEqual(
		{ x: 10, y: 20 },
	);
	expect(runtime.flow.getEdge("e")).toBe(edge);
	runtime.configure({ nodes });
	expect(runtime.store.getState()).toBe(before);
	runtime.flow.setNodes((items) =>
		items.map((item) => (item.id === "a" ? { ...item, selected: true } : item)),
	);
	expect(runtime.flow.getInternalNode("b")).toBe(before.nodeLookup.get("b"));
	expect(runtime.flow.getNode("a")?.selected).toBe(true);
	expect(runtime.store.getState().panZoom).toBeNull();
});

test("camera updates do not notify graph-only subscriptions or rebuild geometry", async () => {
	const runtime = createBoardRuntime();
	runtime.configure({
		nodes: Array.from({ length: 10_000 }, (_, i) => node(String(i), i * 200)),
	});
	const before = runtime.store.getState();
	let graphUpdates = 0;
	let cameraUpdates = 0;
	const stopGraph = runtime.store.subscribeKeys(
		() => graphUpdates++,
		() => new Set(["nodes", "nodeLookup", "edges", "geometryRevision"]),
	);
	const stopCamera = runtime.store.subscribeKeys(
		() => cameraUpdates++,
		() => new Set(["transform"]),
	);
	for (let x = 1; x <= 60; x++)
		await runtime.setViewport({ x, y: x * 2, zoom: 0.1 }, { notify: false });
	expect(graphUpdates).toBe(0);
	expect(cameraUpdates).toBe(60);
	expect(runtime.store.getState().nodeLookup).toBe(before.nodeLookup);
	expect(runtime.store.getState().geometryRevision).toBe(
		before.geometryRevision,
	);
	stopGraph();
	stopCamera();
});

test("native graph mutations notify the controlled host once and retain fresh lookups", () => {
	const runtime = createBoardRuntime();
	const changes: NodeChange[][] = [];
	runtime.configure({
		nodes: [node("a"), node("b")],
		edges: [{ id: "e", source: "a", target: "b" }],
		onNodesChange: (value) => changes.push(value),
	});
	runtime.store
		.getState()
		.triggerNodeChanges([
			{ id: "a", type: "position", position: { x: 40, y: 60 } },
		]);
	expect(changes).toHaveLength(1);
	expect(runtime.flow.getNode("a")?.position).toEqual({ x: 40, y: 60 });
	runtime.flow.setNodes((nodes) => nodes.filter((item) => item.id !== "b"));
	expect(changes[1]).toEqual([{ id: "b", type: "remove" }]);
	expect(runtime.flow.getInternalNode("b")).toBeUndefined();
	runtime.flow.setEdges([]);
	expect(runtime.flow.getEdge("e")).toBeUndefined();
});

test("viewport initialization, callback lifecycle, and silent gesture updates are separate", async () => {
	const runtime = createBoardRuntime();
	const calls: string[] = [];
	runtime.configure({
		width: 1000,
		height: 600,
		onMoveStart: () => calls.push("start"),
		onMove: () => calls.push("move"),
		onMoveEnd: () => calls.push("end"),
	});
	expect(runtime.flow.viewportInitialized).toBe(false);
	await runtime.setViewport({ x: 0, y: 0, zoom: 1 });
	expect(runtime.flow.viewportInitialized).toBe(true);
	expect(calls).toEqual([]);
	await runtime.setViewport({ x: 20, y: 30, zoom: 8 });
	expect(runtime.flow.getViewport()).toEqual({ x: 20, y: 30, zoom: 3 });
	expect(calls).toEqual(["start", "move", "end"]);
	await runtime.setViewport({ x: 30, y: 40, zoom: 0.1 }, { notify: false });
	expect(calls).toHaveLength(3);
	expect(await runtime.setViewport({ x: Number.NaN, y: 0, zoom: 1 })).toBe(
		false,
	);
	expect(runtime.flow.getViewport()).toEqual({ x: 30, y: 40, zoom: 0.1 });
});

test("coordinate conversion accounts for root offset, zoom, and optional grid", async () => {
	const runtime = createBoardRuntime();
	runtime.configure({
		domNode: {
			getBoundingClientRect: () => ({ left: 100, top: 50 }),
		} as HTMLDivElement,
		snapToGrid: true,
		snapGrid: [10, 20],
	});
	await runtime.setViewport({ x: 20, y: 30, zoom: 2 });
	expect(
		runtime.flow.screenToFlowPosition(
			{ x: 150, y: 150 },
			{ snapToGrid: false },
		),
	).toEqual({ x: 15, y: 35 });
	expect(runtime.flow.screenToFlowPosition({ x: 150, y: 150 })).toEqual({
		x: 20,
		y: 40,
	});
	expect(runtime.flow.flowToScreenPosition({ x: 15, y: 35 })).toEqual({
		x: 150,
		y: 150,
	});
});

test("fit view honors hidden nodes, requested subsets, and asymmetric pixel padding", async () => {
	const runtime = createBoardRuntime();
	const nodes = [
		node("a", 100, 200),
		node("b", 500, 400),
		{ ...node("hidden", 1e6, 1e6), hidden: true },
	];
	runtime.configure({ nodes, width: 1200, height: 700 });
	const padding = {
		top: "40px",
		right: "200px",
		bottom: "20px",
		left: "10px",
	} as const;
	expect(await runtime.flow.fitView({ nodes: [{ id: "b" }], padding })).toBe(
		true,
	);
	expect(runtime.flow.getViewport()).toEqual(
		getViewportForBounds(
			{ x: 500, y: 400, width: 150, height: 80 },
			1200,
			700,
			0.02,
			3,
			padding,
		),
	);
	await runtime.flow.fitView();
	expect(runtime.flow.getViewport().zoom).toBeGreaterThan(1);
	expect(runtime.flow.getNodesBounds(["a", "b"])).toEqual({
		x: 100,
		y: 200,
		width: 550,
		height: 280,
	});
});

test("rich DOM measurements update handles once and preserve them during a move", () => {
	const runtime = createBoardRuntime();
	const notifications: NodeChange[][] = [];
	runtime.configure({
		nodes: [node("a")],
		transform: [0, 0, 0.5],
		onNodesChange: (changes) => notifications.push(changes),
	});
	const handle = {
		dataset: { nodeid: "a", handleid: "out", handlepos: "right" },
		getBoundingClientRect: () => ({ left: 149, top: 119, width: 4, height: 4 }),
	};
	const element = {
		isConnected: true,
		offsetWidth: 200,
		offsetHeight: 100,
		getBoundingClientRect: () => ({
			left: 50,
			top: 100,
			width: 100,
			height: 50,
		}),
		querySelectorAll: (selector: string) =>
			selector.endsWith(".source") ? [handle] : [],
	} as unknown as HTMLDivElement;
	const updates = new Map([
		["a", { id: "a", nodeElement: element, force: true }],
	]);
	runtime.store.getState().updateNodeInternals(updates);
	const measured = runtime.flow.getInternalNode("a");
	expect(measured?.measured).toEqual({ width: 200, height: 100 });
	expect(measured?.internals.handleBounds?.source?.[0]).toMatchObject({
		id: "out",
		x: 198,
		y: 38,
		width: 8,
		height: 8,
		position: Position.Right,
	});
	expect(notifications).toHaveLength(1);
	const revision = runtime.store.getState().geometryRevision;
	const measuredRevision = runtime.store.getState().measuredGeometryRevision;
	runtime.store.getState().updateNodeInternals(updates);
	expect(runtime.store.getState().geometryRevision).toBe(revision);
	expect(runtime.store.getState().measuredGeometryRevision).toBe(
		measuredRevision,
	);
	runtime.store
		.getState()
		.triggerNodeChanges([
			{ id: "a", type: "position", position: { x: 15, y: 25 } },
		]);
	expect(runtime.flow.getInternalNode("a")?.internals.handleBounds).toEqual(
		measured?.internals.handleBounds,
	);
	// Auto-layout replaces public records and can omit transient measurements.
	runtime.configure({
		nodes: [{ id: "a", position: { x: 700, y: 300 }, data: {} }],
	});
	expect(runtime.flow.getInternalNode("a")?.internals.positionAbsolute).toEqual(
		{ x: 700, y: 300 },
	);
	expect(runtime.flow.getInternalNode("a")?.measured).toEqual({
		width: 200,
		height: 100,
	});
	expect(runtime.flow.getInternalNode("a")?.internals.handleBounds).toEqual(
		measured?.internals.handleBounds,
	);
	expect(runtime.store.getState().measuredGeometryRevision).toBe(
		measuredRevision,
	);
	// Collapsing rows can move a port without changing the card dimensions.
	handle.getBoundingClientRect = () => ({
		left: 149,
		top: 109,
		width: 4,
		height: 4,
	});
	runtime.store.getState().updateNodeInternals(updates);
	expect(runtime.store.getState().measuredGeometryRevision).toBe(
		measuredRevision + 1,
	);
	expect(
		runtime.store.getState().domMeasuredNodeLookup.get("a")?.internals
			.handleBounds?.source?.[0].y,
	).toBe(18);
});

test("catalog identity changes refresh measured snapshots even when geometry is unchanged", () => {
	const runtime = createBoardRuntime();
	const element = {
		isConnected: true,
		offsetWidth: 150,
		offsetHeight: 80,
		getBoundingClientRect: () => ({ left: 0, top: 0, width: 150, height: 80 }),
		querySelectorAll: () => [],
	} as unknown as HTMLDivElement;
	const updates = new Map([
		["a", { id: "a", nodeElement: element, force: true }],
	]);
	const first = { label: "Before" };
	const second = { label: "After" };
	runtime.configure({ nodes: [{ ...node("a"), data: { node: first } }] });
	runtime.store.getState().updateNodeInternals(updates);
	const revision = runtime.store.getState().measuredGeometryRevision;
	runtime.configure({ nodes: [{ ...node("a"), data: { node: second } }] });
	expect(
		runtime.store.getState().domMeasuredNodeLookup.get("a")?.data.node,
	).toBe(first);
	runtime.store.getState().updateNodeInternals(updates);
	expect(
		runtime.store.getState().domMeasuredNodeLookup.get("a")?.data.node,
	).toBe(second);
	expect(runtime.store.getState().measuredGeometryRevision).toBe(revision + 1);
});

test("remeasuring unchanged DOM ports at another camera does not rebuild scene geometry", async () => {
	const runtime = createBoardRuntime();
	runtime.configure({ nodes: [node("a")] });
	let zoom = 1;
	let x = 20;
	let y = 40;
	let noise = 0;
	let pinY = 28;
	const handle = {
		dataset: { nodeid: "a", handleid: "out", handlepos: "right" },
		getBoundingClientRect: () => ({
			left: x + (146 + noise) * zoom,
			top: y + (pinY - 4 + noise) * zoom,
			width: (8 + noise) * zoom,
			height: (8 + noise) * zoom,
		}),
	};
	const element = {
		isConnected: true,
		offsetWidth: 150,
		offsetHeight: 80,
		getBoundingClientRect: () => ({
			left: x,
			top: y,
			width: 150 * zoom,
			height: 80 * zoom,
		}),
		querySelectorAll: (selector: string) =>
			selector.endsWith(".source") ? [handle] : [],
	} as unknown as HTMLDivElement;
	const updates = new Map([
		["a", { id: "a", nodeElement: element, force: true }],
	]);
	runtime.store.getState().updateNodeInternals(updates);
	const measurements = runtime.store.getState().domMeasuredNodeLookup;
	const revision = runtime.store.getState().measuredGeometryRevision;
	for (const nextZoom of [0.02, 0.1, 0.73, 1.25, 3]) {
		zoom = nextZoom;
		x += 300;
		y -= 100;
		noise = 0.006;
		await runtime.setViewport({ x, y, zoom }, { notify: false });
		runtime.store.getState().updateNodeInternals(updates);
		expect(runtime.store.getState().domMeasuredNodeLookup).toBe(measurements);
		expect(runtime.store.getState().measuredGeometryRevision).toBe(revision);
	}
	pinY += 15;
	runtime.store.getState().updateNodeInternals(updates);
	expect(runtime.store.getState().measuredGeometryRevision).toBe(revision + 1);
	expect(
		runtime.store.getState().domMeasuredNodeLookup.get("a")?.internals
			.handleBounds?.source?.[0].y,
	).toBeCloseTo(39.006, 3);
});

test("controlled and programmatic selections notify once per selected-ID change", () => {
	const runtime = createBoardRuntime();
	const selected: string[][] = [];
	runtime.configure({
		nodes: [node("a"), node("b")],
		edges: [],
		onSelectionChange: ({ nodes, edges }) =>
			selected.push([...nodes, ...edges].map((item) => item.id)),
	});
	runtime.flow.setNodes((nodes) =>
		nodes.map((item) => ({ ...item, selected: item.id === "a" })),
	);
	runtime.configure({
		nodes: runtime.flow
			.getNodes()
			.map((item) => ({ ...item, position: { x: 10, y: 20 } })),
	});
	runtime.flow.setEdges([
		{ id: "e", source: "a", target: "b", selected: true },
	]);
	runtime.store
		.getState()
		.triggerNodeChanges([{ id: "a", type: "select", selected: false }]);
	expect(selected).toEqual([["a"], ["a", "e"], ["e"]]);
});

test("legacy bridge delegates while attached and preserves the latest camera on return", async () => {
	const runtime = createBoardRuntime();
	const legacy = createBoardRuntime();
	legacy.configure({
		nodes: [node("legacy", 400, 500)],
		edges: [],
		width: 800,
		height: 600,
	});
	const detach = runtime.attachLegacy(
		legacy.flow,
		legacy.store as unknown as Parameters<BoardRuntime["attachLegacy"]>[1],
	);
	expect(runtime.store.getState().renderer).toBe("react-flow");
	let geometryNotifications = 0;
	const stop = runtime.store.subscribeKeys(
		() => geometryNotifications++,
		() => new Set(["nodeLookup"]),
	);
	const held = legacy.store.getState().nodeLookup.get("legacy");
	if (!held) throw new Error("Missing legacy node");
	legacy.store.getState().nodeLookup.set("legacy", {
		...held,
		measured: { width: 180, height: 95 },
	});
	legacy.store.setState({ geometryRevision: 2 });
	expect(geometryNotifications).toBe(1);
	expect(runtime.store.getState().nodeLookup.get("legacy")?.measured).toEqual({
		width: 180,
		height: 95,
	});
	const mapBeforeCamera = runtime.store.getState().nodeLookup;
	await runtime.flow.setViewport({ x: 100, y: 200, zoom: 0.5 });
	// Viewport initialization is a separate state update; steady camera movement stays graph-free.
	const graphCount = geometryNotifications;
	const stableMap = runtime.store.getState().nodeLookup;
	await runtime.flow.setViewport({ x: 101, y: 201, zoom: 0.5 });
	expect(geometryNotifications).toBe(graphCount);
	expect(runtime.store.getState().nodeLookup).toBe(stableMap);
	await runtime.flow.setViewport({ x: 100, y: 200, zoom: 0.5 });
	stop();
	expect(legacy.flow.getViewport()).toEqual({ x: 100, y: 200, zoom: 0.5 });
	runtime.flow.setNodes([node("replacement", 300, 100)]);
	expect(legacy.flow.getNodes()[0].id).toBe("replacement");
	detach();
	expect(runtime.store.getState().renderer).toBe("egui");
	expect(runtime.flow.getViewport()).toEqual({ x: 100, y: 200, zoom: 0.5 });
	expect(runtime.flow.getNode("replacement")?.position).toEqual({
		x: 300,
		y: 100,
	});
	await runtime.setViewport({ x: 10, y: 20, zoom: 1 }, { notify: false });
	expect(legacy.flow.getViewport().zoom).toBe(0.5);
	legacy.configure({ nodes: [] });
	expect(runtime.flow.getNodes()).toHaveLength(1);
});

test("legacy cameras below React Flow's zoom floor are clamped about the screen centre", async () => {
	const runtime = createBoardRuntime();
	const legacy = createBoardRuntime();
	legacy.configure({ width: 800, height: 600, minZoom: 0.1, maxZoom: 3 });
	const detach = runtime.attachLegacy(
		legacy.flow,
		legacy.store as unknown as Parameters<BoardRuntime["attachLegacy"]>[1],
	);
	// A native camera restored at 0.02 would otherwise mount the whole board in DOM.
	await runtime.flow.setViewport({ x: 0, y: 0, zoom: 0.02 });
	expect(legacy.flow.getViewport()).toEqual({ x: -1600, y: -1200, zoom: 0.1 });
	expect(legacy.flow.screenToFlowPosition({ x: 400, y: 300 })).toEqual({
		x: 20_000,
		y: 15_000,
	});
	await runtime.flow.setCenter(0, 0, { zoom: 8 });
	expect(legacy.flow.getViewport().zoom).toBe(3);
	await runtime.flow.setViewport({ x: 5, y: 6, zoom: 0.5 });
	expect(legacy.flow.getViewport()).toEqual({ x: 5, y: 6, zoom: 0.5 });
	detach();
});

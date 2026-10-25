import { describe, expect, test } from "bun:test";
import type { INode } from "../schema/flow/node";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../schema/flow/pin";
import {
	type FlowSceneEdge,
	type FlowSceneNode,
	type Palette,
	collectDomNodeIds,
	edgeNeedsDom,
	isNativeCanvasNode,
	nodeNeedsDom,
	sceneFromFlow,
} from "./scene";

const palette: Palette = {
	background: [10, 10, 10, 255],
	card: [20, 20, 20, 255],
	foreground: [250, 250, 250, 255],
	muted: [100, 100, 100, 255],
	border: [50, 50, 50, 255],
	primary: [0, 200, 100, 255],
	tertiary: [150, 80, 220, 255],
};

function pin(
	id: string,
	index: number,
	direction: IPinType,
	extra: Partial<IPin> = {},
): IPin {
	return {
		id,
		index,
		name: id,
		friendly_name: id,
		description: "",
		pin_type: direction,
		data_type: IVariableType.String,
		value_type: IValueType.Normal,
		connected_to: [],
		depends_on: [],
		...extra,
	};
}

function node(
	id: string,
	pins: IPin[],
	extra: Record<string, unknown> = {},
): FlowSceneNode {
	return {
		id,
		type: "node",
		position: { x: 10, y: 20 },
		data: {
			node: {
				id,
				name: "example",
				friendly_name: `Node ${id}`,
				pins: Object.fromEntries(pins.map((pin) => [pin.id, pin])),
				...extra,
			},
		},
	};
}

function edge(
	source: string,
	sourceHandle: string,
	target: string,
	targetHandle: string,
): FlowSceneEdge {
	return {
		id: `${sourceHandle}-${targetHandle}`,
		source,
		sourceHandle,
		target,
		targetHandle,
	};
}

describe("production board canvas scene", () => {
	test("persisted repeated pin groups retain add/remove controls without transient flags", () => {
		const connected = { name: "value", depends_on: ["source"] };
		const graph = node("repeated", [
			pin("a", 1, IPinType.Input, connected),
			pin("b", 2, IPinType.Input, connected),
		]);
		expect(nodeNeedsDom(graph)).toBe(true);
		expect(collectDomNodeIds([graph]).has(graph.id)).toBe(true);
		const separateDirections = node("separate", [
			pin("a", 1, IPinType.Input, connected),
			pin("b", 1, IPinType.Output, { name: "value", connected_to: ["sink"] }),
		]);
		expect(nodeNeedsDom(separateDirections)).toBe(false);
	});
	test("invalid or missing position coordinates never produce an invalid WASM scene", () => {
		const invalid = node("invalid", []);
		invalid.position = { x: Number.NaN, y: Number.POSITIVE_INFINITY };
		const stale = node("stale", []);
		stale.internals = {
			positionAbsolute: { x: Number.NaN, y: Number.NEGATIVE_INFINITY },
		};
		const boundary: FlowSceneNode = {
			id: "layer-return",
			type: "layerInnerNode",
			position: {} as FlowSceneNode["position"],
			data: { type: 1, layer: { id: "layer", name: "Layer", pins: {} } },
		};
		const scene = sceneFromFlow([invalid, stale, boundary], [], palette);
		expect(scene.nodes.map(({ x, y }) => [x, y])).toEqual([
			[0, 0],
			[10, 20],
			[200, 0],
		]);
		expect(
			JSON.parse(JSON.stringify(scene)).nodes.every(
				(node: { x: number; y: number }) =>
					Number.isFinite(node.x) && Number.isFinite(node.y),
			),
		).toBe(true);
	});
	test("production controls and state remain mounted without selecting their nodes", () => {
		const baseline = node("baseline", [
			pin("out", 1, IPinType.Output, { connected_to: ["in"] }),
		]);
		expect(nodeNeedsDom(baseline)).toBe(false);
		expect(nodeNeedsDom({ ...baseline, zIndex: 20 })).toBe(false);
		expect(nodeNeedsDom({ ...baseline, zIndex: 30 })).toBe(true);
		expect(nodeNeedsDom({ ...baseline, style: { opacity: 0.15 } })).toBe(true);
		expect(nodeNeedsDom({ ...baseline, className: "preview-member" })).toBe(
			true,
		);
		expect(
			nodeNeedsDom({ ...baseline, style: { width: 220, height: 80 } }),
		).toBe(true);
		for (const extra of [
			{ error: "Runtime failed" },
			{ comment: "Important context" },
			{ icon: "/icons/catalog.svg" },
			{ only_offline: true },
			{ start: true },
			{ wasm: { package_id: "package" } },
			{ fn_refs: { can_reference_fns: true } },
		])
			expect(nodeNeedsDom(node("rich", [], extra))).toBe(true);
		// The canvas paints the latch, the (x) button, checkboxes, enum selects and containers.
		expect(nodeNeedsDom(node("callback", [], { event_callback: true }))).toBe(
			false,
		);
		const slow = node("slow", [], { long_running: true });
		expect(nodeNeedsDom(slow)).toBe(false);
		expect(sceneFromFlow([slow], [], palette).nodes[0].longRunning).toBe(true);
		expect(sceneFromFlow([baseline], [], palette).nodes[0].longRunning).toBe(
			undefined,
		);
		expect(nodeNeedsDom(node("latch", [pin("out", 1, IPinType.Output)]))).toBe(
			false,
		);
		for (const editor of [
			pin("plain", 1, IPinType.Input),
			pin("bool", 1, IPinType.Input, { data_type: IVariableType.Boolean }),
			pin("choice", 1, IPinType.Input, {
				options: { valid_values: ["first", "second"] },
			}),
			pin("list", 1, IPinType.Output, { value_type: IValueType.Array }),
		])
			expect(nodeNeedsDom(node("editor", [editor]))).toBe(false);
		// Pickers, dynamic families and glyphs outside the canvas font keep the DOM editor.
		for (const editor of [
			pin("fn_ref", 1, IPinType.Input),
			pin("_flow_user_sub", 1, IPinType.Input),
			pin("grow", 1, IPinType.Input, { dynamic: true }),
			pin("choice", 1, IPinType.Input, {
				options: { valid_values: ["一"] },
				default_value: Array.from(new TextEncoder().encode('"一"')),
			}),
		])
			expect(nodeNeedsDom(node("editor", [editor]))).toBe(true);
		for (const data of [
			{ remoteSelections: [{ userId: "peer" }] },
			{ remoteEditors: [{ userId: "peer" }] },
			{ isUnavailable: true },
			{ functionCache: { ttl: 60 } },
		])
			expect(
				nodeNeedsDom({ ...baseline, data: { ...baseline.data, ...data } }),
			).toBe(true);
		// A collaborator's run is painted by the canvas, like a local one.
		expect(
			nodeNeedsDom({
				...baseline,
				data: { ...baseline.data, remoteExecuting: true },
			}),
		).toBe(false);
	});

	test("catalog icons render natively while other icon sources keep the DOM node", () => {
		const wired = [pin("out", 1, IPinType.Output, { connected_to: ["in"] })];
		const catalog = node("catalog", wired, { icon: "/flow/icons/box.svg" });
		expect(nodeNeedsDom(catalog)).toBe(false);
		expect(sceneFromFlow([catalog], [], palette).nodes[0].icon).toBe(
			"/flow/icons/box.svg",
		);
		for (const icon of [
			"https://cdn.example.com/box.svg",
			"/flow/icons/box.png",
			"data:image/svg+xml,<svg/>",
		]) {
			const foreign = node("foreign", wired, { icon });
			expect(nodeNeedsDom(foreign)).toBe(true);
			expect(sceneFromFlow([foreign], [], palette).nodes[0].icon).toBe(
				undefined,
			);
		}
	});

	test("running a board keeps its nodes native; only selection and DOM-only chrome mount editors", () => {
		const nodes = ["idle", "running", "selected"].map((id) => node(id, []));
		nodes[2].selected = true;
		expect([...collectDomNodeIds(nodes)]).toEqual(["selected"]);
		expect([...collectDomNodeIds(nodes, { forceDomText: true })]).toEqual([
			"idle",
			"running",
			"selected",
		]);
		const ids = collectDomNodeIds(nodes);
		expect(
			sceneFromFlow(nodes, [], palette, { domNodeIds: ids })
				.nodes.filter((node) => node.dom)
				.map((node) => node.id),
		).toEqual([...ids]);
	});

	test("native edges defer animations, path variants and decorations to the original SVG", () => {
		const base = {
			...edge("a", "out", "b", "in"),
			type: "execution",
			data: { reduceMotion: true },
			style: { stroke: "var(--foreground)" },
		};
		expect(edgeNeedsDom(base)).toBe(false);
		expect(edgeNeedsDom({ ...base, data: {} })).toBe(true);
		expect(edgeNeedsDom({ ...base, data: {} }, true)).toBe(false);
		for (const change of [
			{ data: { pathType: "straight", reduceMotion: true } },
			{ data: { pathType: "step", reduceMotion: true } },
			{ animated: true },
			{ className: "opacity-10" },
			{ zIndex: 30 },
			{ markerEnd: "url(#arrow)" },
			{ label: "Runs after" },
			{ style: { stroke: "var(--destructive)" } },
			{ style: { opacity: 0.2 } },
		])
			expect(edgeNeedsDom({ ...base, ...change })).toBe(true);
	});

	test("the production flowNode alias preserves real geometry", () => {
		const aliased = { ...node("alias", []), type: "flowNode" };
		expect(isNativeCanvasNode(aliased)).toBe(true);
		expect(sceneFromFlow([aliased], [], palette).nodes[0].id).toBe("alias");
	});

	test("explicit theme strokes keep their color when an edge is selected", () => {
		const source = node("a", [pin("out", 1, IPinType.Output)]);
		const target = node("b", [pin("in", 1, IPinType.Input)]);
		const selected = {
			...edge("a", "out", "b", "in"),
			selected: true,
			type: "data",
			style: { stroke: "var(--pin-boolean)" },
		};
		const scene = sceneFromFlow([source, target], [selected], palette, {
			pinColors: { String: [1, 2, 3, 255], Boolean: [4, 5, 6, 255] },
			webkitLite: true,
		});
		expect(scene.edges[0]).toMatchObject({
			selected: true,
			color: [4, 5, 6, 255],
			lite: true,
			dom: false,
		});
	});

	test("dynamic pin families keep canonical ids, indices and connection endpoints", () => {
		const source = node("source", [pin("out", 1, IPinType.Output)]);
		const dynamic = node("dynamic", [
			pin("second", 2, IPinType.Input, { name: "item", depends_on: ["out"] }),
			pin("third", 3, IPinType.Input, { name: "item" }),
			pin("first", 1, IPinType.Input, { name: "item" }),
		]);
		const before = JSON.stringify(dynamic);
		const scene = sceneFromFlow(
			[source, dynamic],
			[edge("source", "out", "dynamic", "second")],
			palette,
		);
		expect(scene.nodes[1].pins.map((pin) => [pin.id, pin.y])).toEqual([
			["first", 29],
			["second", 44],
			["third", 59],
		]);
		// The fourth row reserves the production family's add-pin control, without inventing a graph pin.
		expect(scene.nodes[1].height).toBe(98);
		expect(scene.edges[0].target).toEqual({
			nodeId: "dynamic",
			pinId: "second",
		});
		expect(JSON.stringify(dynamic)).toBe(before);
	});

	test("collapsed pins retain execution and connected pins and repack only presentation slots", () => {
		const graphNode = node(
			"collapsed",
			[
				pin("exec", 1, IPinType.Input, { data_type: IVariableType.Execution }),
				pin("loose", 2, IPinType.Input),
				pin("wired", 5, IPinType.Input, { depends_on: ["other"] }),
				pin("out", 4, IPinType.Output, { connected_to: ["target"] }),
			],
			{ pins_collapsed: true },
		);
		const before = JSON.stringify(graphNode);
		const compact = sceneFromFlow([graphNode], [], palette).nodes[0];
		expect(compact.pins.map((pin) => [pin.id, pin.y])).toEqual([
			["exec", 29],
			["wired", 44],
			["out", 29],
		]);
		const expanded = sceneFromFlow([graphNode], [], palette, {
			collapsed: new Map([["collapsed", false]]),
		}).nodes[0];
		expect(expanded.pins.find((pin) => pin.id === "wired")?.y).toBe(89);
		expect(expanded.pins.some((pin) => pin.id === "loose")).toBe(true);
		expect(JSON.stringify(graphNode)).toBe(before);
	});

	test("call functions hide their bound layer pin and compact remaining input rows without mutation", () => {
		const call = node(
			"call",
			[
				pin("ref", 1, IPinType.Input, { name: "function_layer_id" }),
				pin("exec", 2, IPinType.Input, {
					name: "exec_in",
					data_type: IVariableType.Execution,
				}),
				pin("arg", 3, IPinType.Input),
				pin("result", 2, IPinType.Output),
			],
			{ name: "control_call_function" },
		);
		call.type = "callFunctionNode";
		const before = JSON.stringify(call);
		const scene = sceneFromFlow([call], [], palette);
		expect(scene.nodes[0].kind).toBe("function");
		expect(
			scene.nodes[0].pins.map((pin) => [pin.id, pin.y, pin.label]),
		).toEqual([
			["exec", 29, ""],
			["arg", 44, "arg"],
			["result", 44, "result"],
		]);
		expect(JSON.stringify(call)).toBe(before);
	});

	test("layer proxies retain their visible node ids and already-inverted boundary direction", () => {
		const boundary: FlowSceneNode = {
			id: "layer-input",
			type: "layerInnerNode",
			position: { x: 0, y: 0 },
			data: {
				type: 0,
				layer: {
					id: "layer",
					name: "Function",
					pins: { entry: pin("entry", 1, IPinType.Output) },
				},
			},
		};
		const proxy: FlowSceneNode = {
			id: "child-layer",
			type: "layerNode",
			position: { x: 300, y: 0 },
			data: {
				layer: {
					id: "child-layer",
					name: "Group",
					pins: { original: pin("original", 1, IPinType.Input) },
				},
			},
		};
		const before = JSON.stringify(boundary);
		const scene = sceneFromFlow(
			[boundary, proxy],
			[edge("layer-input", "entry", "child-layer", "original")],
			palette,
		);
		expect(scene.nodes[0].pins[0].direction).toBe("output");
		expect(scene.nodes[0].label).toBe('"Function" Start');
		expect(scene.edges[0]).toMatchObject({
			source: { nodeId: "layer-input", pinId: "entry" },
			target: { nodeId: "child-layer", pinId: "original" },
		});
		expect(
			sceneFromFlow([boundary], [], palette).nodes[0].pins[0].direction,
		).toBe("output");
		expect(JSON.stringify(boundary)).toBe(before);
	});

	test("function-reference handles have their own ids, geometry and edge kind", () => {
		const from = node("from", [], {
			fn_refs: {
				can_reference_fns: true,
				can_be_referenced_by_fns: false,
				fn_refs: ["to"],
			},
		});
		const to = node("to", [], {
			fn_refs: {
				can_reference_fns: false,
				can_be_referenced_by_fns: true,
				fn_refs: [],
			},
		});
		const refEdge = {
			...edge("from", "ref_out_from", "to", "ref_in_to"),
			type: "veil",
			selected: true,
		};
		const scene = sceneFromFlow([from, to], [refEdge], palette, {
			referenceColor: [1, 2, 3, 255],
		});
		// The DOM puts the referenced square in the header's right end and the reference
		// square on the bottom border, both one border width inside the card.
		expect(scene.nodes[0].pins[0]).toMatchObject({
			id: "ref_out_from",
			direction: "output",
			x: 75,
			y: scene.nodes[0].height - 1,
			kind: "reference",
		});
		expect(scene.nodes[1].pins[0]).toMatchObject({
			id: "ref_in_to",
			direction: "input",
			x: 141,
			y: 9,
		});
		expect(scene.edges[0]).toMatchObject({
			kind: "reference",
			selected: true,
			color: [1, 2, 3, 255],
		});
	});

	test("measured bounds and handle centers override fallback geometry in graph coordinates", () => {
		const measured = node("measured", [pin("input", 1, IPinType.Input)]);
		measured.measured = { width: 180, height: 90 };
		measured.internals = { positionAbsolute: { x: 400, y: 500 } };
		const scene = sceneFromFlow([measured], [], palette, {
			geometry: new Map([
				[
					"measured",
					{
						width: 200,
						handles: [
							{
								id: "input",
								direction: "input",
								x: -6,
								y: 33,
								width: 12,
								height: 12,
							},
						],
					},
				],
			]),
		});
		expect(scene.nodes[0]).toMatchObject({
			x: 400,
			y: 500,
			width: 200,
			height: 90,
		});
		expect(scene.nodes[0].pins[0]).toMatchObject({ x: 0, y: 39 });
	});

	test("reroutes use compact dimensions and centered pins, and start-node inputs stay hidden", () => {
		const reroute = node(
			"route",
			[pin("in", 1, IPinType.Input), pin("out", 1, IPinType.Output)],
			{ name: "reroute", pins_collapsed: true },
		);
		const start = node(
			"start",
			[pin("hidden", 1, IPinType.Input), pin("visible", 1, IPinType.Output)],
			{ start: true },
		);
		const scene = sceneFromFlow([reroute, start], [], palette);
		expect(scene.nodes[0]).toMatchObject({
			width: 16,
			height: 12,
			kind: "reroute",
		});
		expect(scene.nodes[0].pins.map((pin) => [pin.x, pin.y, pin.label])).toEqual(
			[
				[1, 6, ""],
				[15, 6, ""],
			],
		);
		expect(scene.nodes[1].pins.map((pin) => pin.id)).toEqual(["visible"]);
		expect(scene.nodes[1].kind).toBe("event");
	});

	test("comments and media stay with the DOM host, and dangling edges get no invented endpoints", () => {
		const graphNode = node("graph", [pin("out", 1, IPinType.Output)]);
		const comment = {
			id: "comment",
			type: "commentNode",
			position: { x: 0, y: 0 },
			data: { comment: { content: "rich" } },
		};
		const media = { ...comment, id: "media", type: "mediaNode" };
		expect(isNativeCanvasNode(comment)).toBe(false);
		expect(isNativeCanvasNode(media)).toBe(false);
		expect(isNativeCanvasNode({ type: "node", data: {} })).toBe(false);
		const scene = sceneFromFlow(
			[graphNode, comment, media],
			[edge("graph", "out", "media", "missing")],
			palette,
		);
		expect(scene.nodes.map((node) => node.id)).toEqual(["graph"]);
		expect(scene.edges).toEqual([]);
	});

	test("resolved type colors, label caps and selection are retained without browser APIs", () => {
		const typed = node("typed", [
			pin("short", 1, IPinType.Input),
			pin("long", 1, IPinType.Output, {
				friendly_name: "A considerably longer output label",
			}),
		]);
		typed.selected = true;
		typed.data.isUnavailable = true;
		const scene = sceneFromFlow([typed], [], palette, {
			pinColors: { String: [123, 45, 67, 255] },
		});
		expect(scene.nodes[0]).toMatchObject({ selected: true, dimmed: true });
		expect(scene.nodes[0].pins[0].color).toEqual([123, 45, 67, 255]);
		// The two caps share the 134px row. Text loses the label's 4px margin on both sides,
		// and the unconnected input also gives up its 4px gap and 8px (x) button.
		expect(scene.nodes[0].pins[0].editor).toEqual({
			kind: "variable",
			set: false,
		});
		expect(
			(scene.nodes[0].pins[0].labelMaxWidth ?? Number.NaN) +
				(scene.nodes[0].pins[1].labelMaxWidth ?? Number.NaN),
		).toBe(134 - 4 - 12 - 4);
	});

	test("inline editors, containers and the latch carry what the DOM row shows", () => {
		const encode = (value: unknown) =>
			Array.from(new TextEncoder().encode(JSON.stringify(value)));
		const rows = node(
			"rows",
			[
				pin("set", 1, IPinType.Input, { default_value: encode("text") }),
				pin("checked", 2, IPinType.Input, {
					data_type: IVariableType.Boolean,
					default_value: encode(true),
				}),
				pin("unchecked", 3, IPinType.Input, {
					data_type: IVariableType.Boolean,
				}),
				pin("choice", 4, IPinType.Input, {
					options: { valid_values: ["first", "second"] },
					default_value: encode("second"),
				}),
				pin("empty", 5, IPinType.Input, {
					options: { valid_values: ["first"] },
				}),
				pin("wired", 6, IPinType.Input, { depends_on: ["source"] }),
				pin("list", 1, IPinType.Output, { value_type: IValueType.Array }),
				pin("map", 2, IPinType.Output, { value_type: IValueType.HashMap }),
				pin("set-out", 3, IPinType.Output, {
					value_type: IValueType.HashSet,
					connected_to: ["target"],
				}),
			],
			{ pins_collapsed: true },
		);
		const scene = sceneFromFlow([rows], [], palette).nodes[0];
		const byId = Object.fromEntries(scene.pins.map((pin) => [pin.id, pin]));
		expect(byId.wired).toMatchObject({ label: "wired", y: 29 });
		expect(byId.wired.editor).toBeUndefined();
		expect(byId["set-out"]).toMatchObject({ container: "set", y: 29 });
		expect(scene.latch).toEqual({ collapsed: true, hidden: 7 });
		const expanded = sceneFromFlow([rows], [], palette, {
			collapsed: new Map([["rows", false]]),
		}).nodes[0];
		const open = Object.fromEntries(expanded.pins.map((pin) => [pin.id, pin]));
		expect(expanded.latch).toEqual({ collapsed: false, hidden: 0 });
		expect(open.set.editor).toEqual({ kind: "variable", set: true });
		expect(open.checked.editor).toEqual({ kind: "boolean", checked: true });
		expect(open.unchecked.editor).toEqual({ kind: "boolean", checked: false });
		expect(open.choice).toMatchObject({
			label: "second",
			editor: { kind: "select", placeholder: false },
		});
		expect(open.empty).toMatchObject({
			label: "Select empty",
			editor: { kind: "select", placeholder: true },
		});
		expect(open.list.container).toBe("array");
		expect(open.map.container).toBe("map");
		expect(open.list.editor).toBeUndefined();
	});

	test("header gradients follow the production class precedence", () => {
		const exec = pin("exec", 1, IPinType.Input, {
			data_type: IVariableType.Execution,
		});
		const header = (
			extra: Record<string, unknown>,
			pins = [exec],
			data: Record<string, unknown> = {},
		) => {
			const value = node("header", pins, extra);
			Object.assign(value.data, data);
			return sceneFromFlow([value], [], palette, {
				headerColors: { function: [1, 1, 1, 255], testEvent: [2, 2, 2, 255] },
			}).nodes[0].header;
		};
		expect(header({})).toEqual({ color: palette.card, reversed: false });
		expect(header({}, [pin("data", 1, IPinType.Input)])).toEqual({
			color: palette.tertiary,
			reversed: false,
		});
		expect(header({ event_callback: true })).toEqual({
			color: palette.primary,
			reversed: true,
		});
		// Start nodes hide their inputs; their execution output keeps them non-pure.
		const started = [
			pin("next", 1, IPinType.Output, { data_type: IVariableType.Execution }),
		];
		expect(header({ start: true, friendly_name: "Run" }, started)).toEqual({
			color: palette.primary,
			reversed: false,
		});
		expect(
			header({ start: true, friendly_name: "Test empty cart" }, started),
		).toEqual({ color: [2, 2, 2, 255], reversed: false });
		expect(
			header({ event_callback: true }, [exec], { functionLayerId: "layer" }),
		).toEqual({ color: [1, 1, 1, 255], reversed: false });
	});

	test("pin typography and geometry follow rem sizing while headers keep production fitting", () => {
		const graph = node("fitting", [pin("label", 1, IPinType.Output)]);
		(graph.data.node as INode).friendly_name =
			"A workflow title with substantially more than thirty characters";
		const normal = sceneFromFlow([graph], [], palette).nodes[0];
		const large = sceneFromFlow([graph], [], palette, { rootFontSize: 20 })
			.nodes[0];
		expect(normal.pins[0].fontSize).toBe(9.6);
		expect(large.pins[0].fontSize).toBe(12);
		expect(large.pins[0].y - normal.pins[0].y).toBe(7);
		expect(large.height - normal.height).toBe(7);
		expect(large.headerHeight).toBe(20);
		// A <small> in xyflow's 12px node text, shrunk to its 0.6 floor for long titles.
		expect(normal.labelFontSize).toBeCloseTo(5.76);
		expect(large.labelFontSize).toBe(normal.labelFontSize);
	});

	test("node and reroute corner radii follow the resolved CSS including square styles", () => {
		const nodes = [
			node("ordinary", []),
			node("route", [], { name: "reroute" }),
		];
		expect(
			sceneFromFlow(nodes, [], palette).nodes.map((node) => node.cornerRadius),
		).toEqual([4, 6]);
		expect(
			sceneFromFlow(nodes, [], palette, {
				nodeRadius: 10,
				rerouteRadius: 12,
			}).nodes.map((node) => node.cornerRadius),
		).toEqual([10, 12]);
		expect(
			sceneFromFlow(nodes, [], palette, {
				nodeRadius: 0,
				rerouteRadius: 0,
			}).nodes.map((node) => node.cornerRadius),
		).toEqual([0, 0]);
	});

	test("unsupported glyphs and custom fonts preserve complete browser text rendering", () => {
		const graph = node("translated", [pin("out", 1, IPinType.Output)]);
		(graph.data.node as INode).pins.out.connected_to = ["peer"];
		expect(nodeNeedsDom(graph)).toBe(false);
		(graph.data.node as INode).friendly_name = "日本語の処理";
		expect(nodeNeedsDom(graph)).toBe(true);
		(graph.data.node as INode).friendly_name = "Supported title";
		(graph.data.node as INode).pins.out.friendly_name = "Result 😀";
		expect(nodeNeedsDom(graph)).toBe(true);
		(graph.data.node as INode).pins.out.friendly_name = "Result";
		expect(collectDomNodeIds([graph], { forceDomText: true })).toEqual(
			new Set(["translated"]),
		);
	});
});

import { autoResizeTextScale } from "../../components/flow/auto-resize-text-size";
import {
	isPinAction,
	layoutCollapsiblePins,
} from "../../components/flow/flow-node/pin-collapse";
import {
	type PinEditorKind,
	resolvePinEditorKind,
} from "../../components/flow/flow-pin/pin-editor-kind";
import {
	PIN_LABEL_CAP_SHARED,
	planPinLabelCaps,
} from "../../components/flow/flow-pin/pin-label-caps";
import { isTestEventNode } from "../board-tests";
import type { INode } from "../schema/flow/node";
import type { IPin } from "../schema/flow/pin";
import type { ILayer } from "../schema/flow/run";
import { parseUint8ArrayToJson } from "../uint8";
import { hasNativeGlyphs } from "./font-coverage";
import { isNativeIcon } from "./icons";

export type RGBA = readonly [number, number, number, number];
export type PinDirection = "input" | "output";
export type ConnectionKind = "execution" | "data" | "reference";

export interface Palette {
	background: RGBA;
	card: RGBA;
	foreground: RGBA;
	muted: RGBA;
	border: RGBA;
	primary: RGBA;
	tertiary: RGBA;
	/** Card text; xyflow's dark node color differs from the theme foreground. */
	nodeText?: RGBA;
	/** xyflow's node background, behind container pin icons. */
	nodeBackdrop?: RGBA;
	input?: RGBA;
	/** Unchecked checkbox fill. */
	checkbox?: RGBA;
	primaryForeground?: RGBA;
	mutedBackground?: RGBA;
	/** Status colors for runs, logs and lint marks. */
	destructive?: RGBA;
	success?: RGBA;
	caution?: RGBA;
	warning?: RGBA;
	info?: RGBA;
	remote?: RGBA;
	fatal?: RGBA;
}

export type PinEditor =
	| { kind: "variable"; set: boolean }
	| { kind: "boolean"; checked: boolean }
	/** The pin label holds the select's value or placeholder. */
	| { kind: "select"; placeholder: boolean };

export interface ScenePin {
	id: string;
	direction: PinDirection;
	/** Center in node-local graph coordinates, independent of viewport zoom. */
	x: number;
	y: number;
	label: string;
	/** Text width before truncation, excluding the inline editor. */
	labelMaxWidth?: number;
	fontSize?: number;
	color: RGBA;
	kind: ConnectionKind;
	/** Container handles show their type icon instead of a dot. */
	container?: "array" | "set" | "map";
	/** Inline editor of an unconnected input. */
	editor?: PinEditor;
}

export interface SceneNode {
	id: string;
	x: number;
	y: number;
	width: number;
	height: number;
	label: string;
	labelFontSize?: number;
	headerHeight?: number;
	cornerRadius?: number;
	/** Same-origin catalog icon URL; the host rasterizes it into the canvas. */
	icon?: string;
	/** Header gradient from the card to `color`; reversed runs from the right edge. */
	header?: { color: RGBA; reversed?: boolean };
	/** The strip that hides unconnected pins, with the count it hides. */
	latch?: { collapsed: boolean; hidden: number };
	/** Mirror WebKit's DOM node: flat pins and its text baselines. */
	lite?: boolean;
	/** The clock badge on the node's top-left corner. */
	longRunning?: boolean;
	kind:
		| "event"
		| "function"
		| "pure"
		| "standard"
		| "layer"
		| "boundary"
		| "reroute";
	pins: ScenePin[];
	selected: boolean;
	dimmed: boolean;
	/** The host renders this node's chrome and controls in the DOM. Geometry stays available. */
	dom?: boolean;
}

export interface SceneEndpoint {
	/** Visible node id; pinId remains the canonical pin or function-reference handle id. */
	nodeId: string;
	pinId: string;
}

export interface SceneEdge {
	id: string;
	source: SceneEndpoint;
	target: SceneEndpoint;
	kind: ConnectionKind;
	selected: boolean;
	color: RGBA;
	/** Keep geometry for host interactions while the original edge renders in SVG. */
	dom?: boolean;
	/** WebKit uses the production data edge's single-stroke treatment. */
	lite?: boolean;
}

export interface Scene {
	nodes: SceneNode[];
	edges: SceneEdge[];
}

export interface Overlay {
	connection?: {
		from: [number, number];
		to: [number, number];
		fromDirection: PinDirection;
		kind?: ScenePin["kind"];
		valid?: boolean;
		color: RGBA;
	};
	hoveredPin?: { nodeId: string; pinId: string; valid: boolean };
	selectionRect?: [number, number, number, number];
}

/** Structural inputs also accept React Flow's public and internal node objects. */
export interface FlowSceneNode {
	id: string;
	type?: string;
	position: { x: number; y: number };
	width?: number;
	height?: number;
	measured?: { width?: number; height?: number };
	internals?: { positionAbsolute?: { x: number; y: number } };
	data: Record<string, unknown>;
	selected?: boolean;
	hidden?: boolean;
	className?: string;
	zIndex?: number;
	style?: CSSProperties;
}

export interface FlowSceneEdge {
	id: string;
	source: string;
	target: string;
	sourceHandle?: string | null;
	targetHandle?: string | null;
	type?: string;
	data?: Record<string, unknown>;
	selected?: boolean;
	hidden?: boolean;
	className?: string;
	zIndex?: number;
	animated?: boolean;
	markerStart?: unknown;
	markerEnd?: unknown;
	label?: unknown;
	style?: CSSProperties;
}

export interface MeasuredHandle {
	id: string;
	direction: PinDirection;
	/** Node-local handle bounds; omit width/height when x/y already name its center. */
	x: number;
	y: number;
	width?: number;
	height?: number;
}

export interface MeasuredNodeGeometry {
	width?: number;
	height?: number;
	handles?: readonly MeasuredHandle[];
}

export interface SceneOptions {
	geometry?: ReadonlyMap<string, MeasuredNodeGeometry>;
	/** Resolved theme colors keyed by the domain data type, such as String or Geometry. */
	pinColors?: Readonly<Record<string, RGBA>>;
	referenceColor?: RGBA;
	/** Local pin-collapse overrides for read-only previews. */
	collapsed?: ReadonlyMap<string, boolean>;
	boundaryLabels?: { input: string; output: string };
	domNodeIds?: ReadonlySet<string>;
	webkitLite?: boolean;
	/** Resolved Tailwind violet-500 and cyan-500 header tints. */
	headerColors?: { function: RGBA; testEvent: RGBA };
	rootFontSize?: number;
	nodeRadius?: number;
	rerouteRadius?: number;
}

const GRAPH_TYPES = new Set([
	"node",
	"flowNode",
	"callFunctionNode",
	"layerNode",
	"layerInnerNode",
]);
const PIN_ROW_HEIGHT = 15;
const PIN_LATCH_HEIGHT = 10;

export function isNativeCanvasNode(
	node: Pick<FlowSceneNode, "type" | "data">,
): boolean {
	if (!node.type || !GRAPH_TYPES.has(node.type)) return false;
	const entity =
		node.type === "node" ||
		node.type === "flowNode" ||
		node.type === "callFunctionNode"
			? node.data.node
			: node.data.layer;
	return typeof entity === "object" && entity !== null && "pins" in entity;
}

/** Production components retain interactive, platform, and runtime presentation. */
export function nodeNeedsDom(node: FlowSceneNode): boolean {
	if (!isNativeCanvasNode(node)) return true;
	if (node.type === "layerNode" || node.type === "layerInnerNode") return true;
	// Group previews and other host decorations act on the original DOM wrapper.
	if (
		node.className ||
		Object.keys(node.style ?? {}).length > 0 ||
		(node.zIndex !== undefined && node.zIndex !== 20)
	)
		return true;
	const entity = node.data.node as INode;
	if (
		!hasNativeGlyphs(entity.friendly_name || entity.name || node.id) ||
		Object.values(entity.pins).some(
			(pin) => !hasNativeGlyphs(pin.friendly_name),
		)
	)
		return true;
	if (
		entity.error ||
		entity.comment ||
		(entity.icon && !isNativeIcon(entity.icon)) ||
		entity.start ||
		entity.only_offline ||
		entity.wasm?.package_id ||
		entity.fn_refs?.can_reference_fns ||
		entity.fn_refs?.can_be_referenced_by_fns ||
		node.data.functionCache ||
		node.data.isUnavailable ||
		(Array.isArray(node.data.remoteSelections) &&
			node.data.remoteSelections.length > 0) ||
		(Array.isArray(node.data.remoteEditors) &&
			node.data.remoteEditors.length > 0)
	)
		return true;
	if (entity.name === "reroute") return false;
	const pinGroups = new Set<string>();
	for (const pin of Object.values(entity.pins)) {
		const group = `${pin.pin_type}:${pin.name}`;
		// FlowNode derives add/remove controls from repeated pin names, even when
		// persisted pins do not carry the transient `dynamic` flag.
		if (pinGroups.has(group)) return true;
		pinGroups.add(group);
	}
	return Object.values(entity.pins).some(
		(pin) => pin.dynamic || !hasNativeEditor(pin, entity.name),
	);
}

const NATIVE_EDITORS = new Set<PinEditorKind>([
	"label",
	"plain",
	"boolean",
	"enum",
]);

/** The canvas draws labels, the (x) button, checkboxes and enum selects; pickers stay DOM. */
function hasNativeEditor(pin: IPin, nodeName: string): boolean {
	if (pin.name === "exec_in" || pin.name === "exec_out") return true;
	const kind = resolvePinEditorKind(pin, nodeName);
	if (kind !== "enum") return NATIVE_EDITORS.has(kind);
	const value = parseUint8ArrayToJson(pin.default_value);
	return typeof value !== "string" || hasNativeGlyphs(value);
}

export interface NativePresentationState {
	/** A custom UI font is displayed by the original DOM until its native font is available. */
	forceDomText?: boolean;
}

/**
 * Nodes the DOM renders. Run state, logs, heat and lint marks are painted by the canvas
 * (`collectNodeStatus`), so running a board never moves nodes into the DOM.
 */
export function collectDomNodeIds(
	nodes: readonly FlowSceneNode[],
	state: NativePresentationState = {},
): Set<string> {
	const result = new Set<string>();
	for (const node of nodes) {
		if (state.forceDomText || node.selected || nodeNeedsDom(node))
			result.add(node.id);
	}
	return result;
}

const NATIVE_STROKE =
	/^var\(--(?:foreground|pin-(?:string|integer|float|boolean|struct|generic|pathbuf|byte|date|hashmap|hashset|execution|geometry|fn-ref))\)$/;

/** SVG owns path modes, markers, CSS animations, and arbitrary edge presentation. */
export function edgeNeedsDom(edge: FlowSceneEdge, webkitLite = false): boolean {
	return Boolean(
		(edge.type && !["execution", "data", "veil"].includes(edge.type)) ||
			(edge.data?.pathType &&
				!["default", "bezier"].includes(String(edge.data.pathType))) ||
			edge.animated ||
			edge.className ||
			(edge.zIndex !== undefined && edge.zIndex !== 18) ||
			edge.markerStart ||
			edge.markerEnd ||
			edge.label != null ||
			Object.keys(edge.style ?? {}).some((key) => key !== "stroke") ||
			(edge.style?.stroke && !NATIVE_STROKE.test(String(edge.style.stroke))) ||
			(edge.type === "execution" &&
				!webkitLite &&
				edge.data?.reduceMotion !== true),
	);
}

function resolvedEdgeStroke(
	edge: FlowSceneEdge,
	palette: Palette,
	options: SceneOptions,
): RGBA | undefined {
	const stroke = edge.style?.stroke;
	if (stroke === "var(--foreground)") return palette.foreground;
	if (stroke === "var(--pin-fn-ref)")
		return options.referenceColor ?? palette.tertiary;
	const token = /^var\(--pin-([a-z]+)\)$/.exec(String(stroke))?.[1];
	if (!token) return undefined;
	return Object.entries(options.pinColors ?? {}).find(
		([type]) => type.toLowerCase() === token,
	)?.[1];
}

function positive(...values: (number | undefined)[]): number {
	return (
		values.find(
			(value) => value !== undefined && Number.isFinite(value) && value > 0,
		) ?? 1
	);
}

function finiteCoordinate(...values: (number | undefined)[]): number {
	return (
		values.find(
			(value) => typeof value === "number" && Number.isFinite(value),
		) ?? 0
	);
}

function pinOrder(a: IPin, b: IPin): number {
	return a.index - b.index || a.id.localeCompare(b.id);
}

function nodeKind(
	node: FlowSceneNode,
	entity: INode | ILayer,
): SceneNode["kind"] {
	if (node.type === "layerInnerNode") return "boundary";
	if (node.type === "layerNode") return "layer";
	const data = entity as INode;
	if (data.name === "reroute") return "reroute";
	if (data.start || data.event_callback) return "event";
	if (data.name === "control_call_function" || node.data.functionLayerId)
		return "function";
	return Object.values(data.pins).some((pin) => pin.data_type === "Execution")
		? "standard"
		: "pure";
}

const PIN_CONTAINERS: Partial<Record<string, ScenePin["container"]>> = {
	Array: "array",
	HashSet: "set",
	HashMap: "map",
};
/** Tailwind violet-500 and cyan-500 in sRGB, for themes that do not resolve them. */
const FUNCTION_HEADER: RGBA = [141, 84, 255, 255];
const TEST_EVENT_HEADER: RGBA = [0, 183, 215, 255];

/**
 * PinEdit's row inside its label cap: the label keeps a 4px margin, and an unconnected
 * input adds its editor after a gap (the (x) button, a checkbox, or a select chevron).
 */
function pinRow(
	pin: IPin,
	nodeName: string | undefined,
	cap = PIN_LABEL_CAP_SHARED,
): Pick<ScenePin, "label" | "labelMaxWidth" | "editor"> {
	const label = pin.friendly_name || pin.name;
	const room = cap - 4;
	if (pin.pin_type !== "Input") return { label, labelMaxWidth: room };
	switch (resolvePinEditorKind(pin, nodeName)) {
		case "plain":
			return pin.connected_to.length > 0
				? { label, labelMaxWidth: room }
				: {
						label,
						labelMaxWidth: room - 12,
						editor: { kind: "variable", set: pin.default_value != null },
					};
		case "boolean":
			return {
				label,
				labelMaxWidth: room - 14,
				editor: {
					kind: "boolean",
					checked: parseUint8ArrayToJson(pin.default_value) === true,
				},
			};
		case "enum": {
			const value = parseUint8ArrayToJson(pin.default_value);
			const set = typeof value === "string" && value !== "";
			return {
				label: set ? value : `Select ${pin.friendly_name}`,
				labelMaxWidth: room - 10,
				editor: { kind: "select", placeholder: !set },
			};
		}
		default:
			return { label, labelMaxWidth: room };
	}
}

/**
 * FlowNode stacks every matching gradient class, so the stylesheet order decides:
 * violet over tertiary over primary over cyan, and a rightward gradient over a leftward one.
 */
function headerStyle(
	node: FlowSceneNode,
	entity: INode,
	visible: readonly IPin[],
	palette: Palette,
	options: SceneOptions,
): NonNullable<SceneNode["header"]> {
	const functionLayer = Boolean(node.data.functionLayerId);
	const pure = !visible.some((pin) => pin.data_type === "Execution");
	const start = entity.start === true;
	const testEvent = isTestEventNode(entity);
	const color = functionLayer
		? (options.headerColors?.function ?? FUNCTION_HEADER)
		: pure
			? palette.tertiary
			: entity.event_callback || (start && !testEvent)
				? palette.primary
				: start
					? (options.headerColors?.testEvent ?? TEST_EVENT_HEADER)
					: palette.card;
	return {
		color,
		reversed:
			entity.event_callback === true && !functionLayer && !pure && !start,
	};
}

function repeatedFamilyCount(pins: readonly IPin[]): number {
	let count = 0;
	for (let i = 1; i < pins.length; i++) {
		if (
			pins[i].name === pins[i - 1].name &&
			(i === 1 || pins[i - 2].name !== pins[i].name)
		)
			count++;
	}
	return count;
}

function makeNode(
	node: FlowSceneNode,
	palette: Palette,
	options: SceneOptions,
): SceneNode {
	const ordinary =
		node.type === "node" ||
		node.type === "flowNode" ||
		node.type === "callFunctionNode";
	const entity = (ordinary ? node.data.node : node.data.layer) as
		| INode
		| ILayer;
	const domainNode = ordinary ? (entity as INode) : undefined;
	const kind = nodeKind(node, entity);
	const rootFontSize = positive(options.rootFontSize, 16);
	const firstPinY = rootFontSize * 1.75;
	const reroute = kind === "reroute";
	let pins = Object.values(entity.pins);
	if (domainNode?.name === "control_call_function") {
		let inputIndex = 0;
		pins = pins
			.filter((pin) => pin.name !== "function_layer_id")
			.sort(pinOrder)
			.map((pin) =>
				pin.pin_type === "Input" ? { ...pin, index: ++inputIndex } : pin,
			);
	}
	// parseBoard already inverted boundary pins. Never infer direction from the visible id suffix.
	if (node.type === "layerInnerNode") {
		const inputBoundary =
			node.data.type === 0 ||
			(node.data.type === undefined && node.id.endsWith("-input"));
		pins = pins.filter(
			(pin) => pin.pin_type === (inputBoundary ? "Output" : "Input"),
		);
	}
	const inputs = domainNode?.start
		? []
		: pins.filter((pin) => pin.pin_type === "Input").sort(pinOrder);
	const outputs = pins
		.filter((pin) => pin.pin_type === "Output")
		.sort(pinOrder);
	const collapsed =
		ordinary &&
		!reroute &&
		(options.collapsed?.get(node.id) ?? domainNode?.pins_collapsed === true);
	const layout = layoutCollapsiblePins(inputs, outputs, collapsed);
	const visible = [...layout.inputs, ...layout.outputs].filter(
		(pin): pin is IPin => !isPinAction(pin),
	);
	const extraInputs = ordinary && !collapsed ? repeatedFamilyCount(inputs) : 0;
	const extraOutputs =
		ordinary && !collapsed ? repeatedFamilyCount(outputs) : 0;
	const rows = Math.max(
		1,
		layout.inputs.length + extraInputs,
		layout.outputs.length + extraOutputs,
		...visible.map((pin) => layout.slots[pin.id] ?? pin.index),
	);
	const measured = options.geometry?.get(node.id);
	const width = positive(
		measured?.width,
		node.measured?.width,
		node.width,
		reroute ? 16 : 150,
	);
	const height = positive(
		measured?.height,
		node.measured?.height,
		node.height,
		reroute
			? 12
			: firstPinY +
					rows * PIN_ROW_HEIGHT +
					(ordinary && layout.collapsibleCount > 0 ? PIN_LATCH_HEIGHT : 0),
	);
	const caps = reroute
		? {}
		: planPinLabelCaps(
				layout.inputs,
				layout.outputs,
				layout.slots,
				() => domainNode?.name,
			);
	const measuredHandles = new Map(
		measured?.handles?.map((handle) => [
			`${handle.direction}:${handle.id}`,
			handle,
		]),
	);
	const locate = (pin: ScenePin): ScenePin => {
		const handle = measuredHandles.get(`${pin.direction}:${pin.id}`);
		if (!handle || !Number.isFinite(handle.x) || !Number.isFinite(handle.y))
			return pin;
		return {
			...pin,
			x: handle.x + (handle.width ?? 0) / 2,
			y: handle.y + (handle.height ?? 0) / 2,
		};
	};
	const scenePins: ScenePin[] = visible.map((pin) =>
		locate({
			id: pin.id,
			fontSize: rootFontSize * 0.6,
			direction: pin.pin_type === "Input" ? "input" : "output",
			// DOM handles are centered on the padding box, inside the 1px card border.
			x: pin.pin_type === "Input" ? 1 : width - 1,
			y: reroute
				? height / 2
				: firstPinY +
					1 +
					((layout.slots[pin.id] ?? pin.index) - 1) * PIN_ROW_HEIGHT,
			...(reroute || pin.name === "exec_in" || pin.name === "exec_out"
				? { label: "" }
				: pinRow(pin, domainNode?.name, caps[pin.id])),
			color:
				pin.data_type === "Execution"
					? palette.foreground
					: (options.pinColors?.[pin.data_type] ?? palette.foreground),
			kind: pin.data_type === "Execution" ? "execution" : "data",
			container:
				pin.data_type === "Execution"
					? undefined
					: PIN_CONTAINERS[pin.value_type],
		}),
	);
	const referenceColor = options.referenceColor ?? palette.tertiary;
	if (domainNode?.fn_refs?.can_be_referenced_by_fns)
		scenePins.push(
			locate({
				id: `ref_in_${domainNode.id}`,
				direction: "input",
				x: width - 9,
				y: 9,
				label: "",
				color: referenceColor,
				kind: "reference",
			}),
		);
	if (domainNode?.fn_refs?.can_reference_fns)
		scenePins.push(
			locate({
				id: `ref_out_${domainNode.id}`,
				direction: "output",
				x: width / 2,
				y: height - 1,
				label: "",
				color: referenceColor,
				kind: "reference",
			}),
		);
	let label = domainNode?.friendly_name || entity.name || node.id;
	if (node.type === "layerInnerNode") {
		const inputBoundary =
			node.data.type === 0 ||
			(node.data.type === undefined && node.id.endsWith("-input"));
		label = `"${entity.name}" ${inputBoundary ? (options.boundaryLabels?.input ?? "Start") : (options.boundaryLabels?.output ?? "Return")}`;
	}
	const boundaryX =
		node.type === "layerInnerNode"
			? node.data.type === 0 ||
				(node.data.type === undefined && node.id.endsWith("-input"))
				? -200
				: 200
			: 0;
	return {
		id: node.id,
		x: finiteCoordinate(
			node.internals?.positionAbsolute?.x,
			node.position?.x,
			boundaryX,
		),
		y: finiteCoordinate(
			node.internals?.positionAbsolute?.y,
			node.position?.y,
			0,
		),
		width,
		height,
		label,
		// The title is a <small> in xyflow's fixed 12px node text, so rem sizing never scales it.
		labelFontSize: 9.6 * autoResizeTextScale(label, 30),
		headerHeight: rootFontSize,
		cornerRadius: Math.max(
			0,
			finiteCoordinate(
				reroute ? options.rerouteRadius : options.nodeRadius,
				reroute ? 6 : 4,
			),
		),
		icon:
			domainNode?.icon && isNativeIcon(domainNode.icon)
				? domainNode.icon
				: undefined,
		header: domainNode
			? headerStyle(node, domainNode, visible, palette, options)
			: undefined,
		latch:
			ordinary && !reroute && layout.collapsibleCount > 0
				? { collapsed, hidden: layout.hiddenCount }
				: undefined,
		lite: options.webkitLite === true,
		longRunning: domainNode?.long_running === true || undefined,
		kind,
		pins: scenePins,
		selected: node.selected === true,
		dimmed: node.data.isUnavailable === true || node.data.dimmed === true,
		dom: options.domNodeIds?.has(node.id) ?? nodeNeedsDom(node),
	};
}

/** Projects parsed nodes only; commands and compatibility checks remain with the host. */
export function sceneFromFlow(
	nodes: readonly FlowSceneNode[],
	edges: readonly FlowSceneEdge[],
	palette: Palette,
	options: SceneOptions = {},
): Scene {
	const sceneNodes = nodes
		.filter((node) => !node.hidden && isNativeCanvasNode(node))
		.map((node) => makeNode(node, palette, options));
	const pins = new Map(
		sceneNodes.map((node) => [
			node.id,
			new Map(node.pins.map((pin) => [pin.id, pin])),
		]),
	);
	const sceneEdges: SceneEdge[] = [];
	for (const edge of edges) {
		if (edge.hidden || !edge.sourceHandle || !edge.targetHandle) continue;
		const source = pins.get(edge.source)?.get(edge.sourceHandle);
		const target = pins.get(edge.target)?.get(edge.targetHandle);
		// Missing/DOM-only endpoints are left to the host, never attached to an invented pin.
		if (!source || !target) continue;
		const kind =
			edge.type === "veil" ||
			edge.data?.isFnRef ||
			source.kind === "reference" ||
			target.kind === "reference"
				? "reference"
				: edge.type === "execution" || source.kind === "execution"
					? "execution"
					: "data";
		sceneEdges.push({
			id: edge.id,
			source: { nodeId: edge.source, pinId: edge.sourceHandle },
			target: { nodeId: edge.target, pinId: edge.targetHandle },
			kind,
			selected: edge.selected === true,
			dom: edgeNeedsDom(edge, options.webkitLite),
			lite: options.webkitLite,
			color:
				resolvedEdgeStroke(edge, palette, options) ??
				(kind === "reference"
					? (options.referenceColor ?? palette.tertiary)
					: kind === "execution"
						? palette.foreground
						: source.color),
		});
	}
	return { nodes: sceneNodes, edges: sceneEdges };
}
import type { CSSProperties } from "react";

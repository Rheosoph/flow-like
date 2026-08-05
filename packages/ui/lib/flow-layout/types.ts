import type { ILayer } from "../schema/flow/board";
import type { INode } from "../schema/flow/node";

export type LayoutStyle = "compact" | "expanded" | "balanced";

export type LNodeKind = "exec" | "pure" | "reroute" | "entity";

export interface LayoutEntity {
	id: string;
	coordinates: number[];
}

export interface LayoutComment {
	id: string;
	x: number;
	y: number;
	width: number;
	height: number;
	isLocked?: boolean;
}

export interface AutoLayoutInput {
	layerNodes: INode[];
	layerEntities: LayoutEntity[];
	boardLayers?: Record<string, ILayer>;
	currentLayer: string | undefined;
	/** Real sizes measured by react-flow. Preferred over the CSS formula. */
	nodeSizes?: ReadonlyMap<string, readonly [number, number]>;
	comments?: readonly LayoutComment[];
	/** Restricts layout to these node ids (layout-of-selection). */
	only?: ReadonlySet<string>;
	/**
	 * Boxes the layout must not land on. Used with `only` to keep a scoped
	 * layout off the nodes the user did not select.
	 */
	obstacles?: readonly LayoutBox[];
}

export interface LayoutBox {
	x: number;
	y: number;
	width: number;
	height: number;
}

export interface PinRef {
	id: string;
	index: number;
	offsetY: number;
	isExec: boolean;
}

export type EdgeKind = "exec" | "data";

export interface LEdge {
	from: string;
	to: string;
	fromPin: PinRef;
	toPin: PinRef;
	kind: EdgeKind;
	/** Back edge found during cycle breaking; ignored by ranking and ordering. */
	reversed: boolean;
}

export interface LNode {
	id: string;
	kind: LNodeKind;
	isStart: boolean;
	width: number;
	height: number;
	execIn: PinRef[];
	execOut: PinRef[];
	dataIn: PinRef[];
	dataOut: PinRef[];
	out: LEdge[];
	in: LEdge[];
	fnRefTargets: string[];
	component: number;
	column: number;
	order: number;
	/** For pure nodes: the exec node whose vertical band they hang under. */
	owner: string | null;
	depth: number;
	x: number;
	y: number;
	/** Set once the vertical passes have assigned a final y. */
	placed: boolean;
	/** Reroute moved into the gutter between columns; excluded from packing. */
	parked?: boolean;
}

export interface LGraph {
	nodes: Map<string, LNode>;
	/** The one canonical iteration order. Never iterate the Map for a decision. */
	order: string[];
	edges: LEdge[];
	entityIds: Set<string>;
}

export interface StyleConfig {
	/** Edge-to-edge horizontal gap between columns. */
	hGap: number;
	/** Edge-to-edge vertical gap between nodes in a column. */
	vGap: number;
	/** Gap between the exec spine and the pure-node band below it. */
	pureVGap: number;
	/** Gap between weakly connected components. */
	componentGap: number;
}

export interface LayoutDiagnostics {
	components: Array<{ id: number; nodeIds: string[]; roots: string[] }>;
	columns: Map<string, number>;
	orders: Map<string, number>;
	owners: Map<string, string>;
	unplaced: string[];
}

export interface LayoutResult {
	positions: Map<string, [number, number]>;
	commentPositions: Map<string, [number, number]>;
	reversedEdges: ReadonlyArray<{ from: string; to: string }>;
	diagnostics: LayoutDiagnostics;
}

export function getStyleConfig(style: LayoutStyle): StyleConfig {
	switch (style) {
		case "compact":
			return { hGap: 80, vGap: 40, pureVGap: 24, componentGap: 120 };
		case "expanded":
			return { hGap: 220, vGap: 90, pureVGap: 44, componentGap: 320 };
		default:
			return { hGap: 140, vGap: 60, pureVGap: 32, componentGap: 200 };
	}
}

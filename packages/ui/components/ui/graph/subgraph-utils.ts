import {
	effectiveIdentityColumn,
	endpointIdentity,
	resolveEdgeMapping,
} from "../../../lib/ontology-object-edit";
import type {
	EdgeLabelMapping,
	GraphOverlay,
	LabelStyle,
	NodeLabelMapping,
	SubgraphEdge,
	SubgraphNode,
	SubgraphResult,
} from "../../../state/backend-state/graph-state";

/**
 * Degree-based by default: connectivity is the one visual encoding a graph can
 * always honestly carry, and an 8px floor keeps the icon glyph readable —
 * the old 6px fixed default rendered it at ~2px.
 */
export const DEFAULT_LABEL_STYLE: LabelStyle = {
	color: "#6b7280",
	icon: "circle",
	size: { mode: "by-degree", min: 8, max: 20 },
};

const SYNTHETIC_PALETTE = [
	"#6366f1",
	"#10b981",
	"#f59e0b",
	"#ef4444",
	"#06b6d4",
	"#8b5cf6",
	"#ec4899",
	"#84cc16",
	"#f97316",
	"#14b8a6",
];

/** Stable per-label index so a label keeps its colour across re-renders and data reloads. */
function labelSeed(label: string): number {
	let hash = 0;
	for (let index = 0; index < label.length; index += 1) {
		hash = (hash * 31 + label.charCodeAt(index)) | 0;
	}
	return Math.abs(hash);
}

export function enrichSubgraphWithStyles(
	result: SubgraphResult,
	overlay: GraphOverlay,
): SubgraphResult {
	const nodeStyleMap = new Map(
		overlay.nodes.map((node) => [node.label, node.style]),
	);
	const edgeStyleMap = new Map(
		overlay.edges.map((edge) => [edge.label, edge.style]),
	);

	return {
		...result,
		nodes: result.nodes.map((node) => ({
			...node,
			style: nodeStyleMap.get(node.label) ?? node.style ?? DEFAULT_LABEL_STYLE,
		})),
		edges: result.edges.map((edge) => ({
			...edge,
			style: edgeStyleMap.get(edge.label) ?? edge.style ?? DEFAULT_LABEL_STYLE,
		})),
	};
}

export function mergeSubgraphData(
	current: SubgraphResult | null,
	incoming: SubgraphResult,
): SubgraphResult {
	if (!current) return incoming;

	const nodeIds = new Set(current.nodes.map((node) => node.id));
	const edgeIds = new Set(current.edges.map((edge) => edge.id));
	const warnings = Array.from(
		new Set([...(current.warnings ?? []), ...(incoming.warnings ?? [])]),
	);

	return {
		nodes: [
			...current.nodes,
			...incoming.nodes.filter((node) => !nodeIds.has(node.id)),
		],
		edges: [
			...current.edges,
			...incoming.edges.filter((edge) => !edgeIds.has(edge.id)),
		],
		truncated: current.truncated || incoming.truncated,
		...(warnings.length > 0 ? { warnings } : {}),
	};
}

export function collectSubtree(
	parentNodeId: string,
	childMap: Map<string, Set<string>>,
	acc: Set<string>,
): void {
	const children = childMap.get(parentNodeId);
	if (!children) return;
	for (const childId of children) {
		if (acc.has(childId)) continue;
		acc.add(childId);
		collectSubtree(childId, childMap, acc);
	}
}

export function removeSubtree(
	current: SubgraphResult | null,
	removed: Set<string>,
): SubgraphResult | null {
	if (!current || removed.size === 0) return current;
	return {
		...current,
		nodes: current.nodes.filter((node) => !removed.has(node.id)),
		edges: current.edges.filter(
			(edge) => !removed.has(edge.source) && !removed.has(edge.target),
		),
	};
}

function sameJsonValue(left: unknown, right: unknown): boolean {
	return JSON.stringify(left ?? null) === JSON.stringify(right ?? null);
}

/** The server's caption rule: a string display value, else the raw id. */
function nodeCaption(
	node: SubgraphNode,
	displayColumn: string,
	props: Record<string, unknown>,
): string {
	const display = props[displayColumn];
	if (typeof display === "string") return display;
	const prefix = `${node.label}:`;
	return node.id.startsWith(prefix) ? node.id.slice(prefix.length) : node.id;
}

function patchNodeProps(
	node: SubgraphNode,
	patch: Record<string, unknown>,
	overlay: GraphOverlay | null,
): SubgraphNode {
	const props = { ...node.props, ...patch };
	const displayColumn = overlay?.nodes.find(
		(mapping) => mapping.label === node.label,
	)?.display_column;
	const captionChanged =
		displayColumn !== undefined &&
		Object.hasOwn(patch, displayColumn) &&
		!sameJsonValue(node.props[displayColumn], props[displayColumn]);
	return {
		...node,
		props,
		...(captionChanged
			? { caption: nodeCaption(node, displayColumn, props) }
			: {}),
	};
}

/**
 * Swaps in a node's freshly saved row without touching the id set, so the
 * canvas keeps its layout. Only keys the node already shows, or that were
 * just edited, are taken from `fresh`. Never use `mergeSubgraphData` for this:
 * it keeps the first copy of a node.
 */
export function patchSubgraphNode(
	data: SubgraphResult | null,
	nodeId: string,
	fresh: Record<string, unknown>,
	overlay: GraphOverlay | null,
	changedKeys: readonly string[],
): SubgraphResult | null {
	const index = data?.nodes.findIndex((node) => node.id === nodeId) ?? -1;
	if (!data || index < 0) return data;
	const node = data.nodes[index];
	const edited = new Set(changedKeys);
	const patch = Object.fromEntries(
		Object.entries(fresh).filter(
			([key]) => Object.hasOwn(node.props, key) || edited.has(key),
		),
	);
	const nodes = [...data.nodes];
	nodes[index] = patchNodeProps(node, patch, overlay);
	return { ...data, nodes };
}

/** A row just saved through one graph element. */
export interface SavedRow {
	table: string;
	/** The row as the edited element held it before the save, identity included. */
	known: Readonly<Record<string, unknown>>;
	/** The row as the server returned it. */
	fresh: Readonly<Record<string, unknown>>;
}

/** Column sets that single out one row of `table`. */
function rowKeyColumns(overlay: GraphOverlay, table: string): string[][] {
	const keys: string[][] = [];
	for (const mapping of overlay.nodes) {
		if (mapping.table !== table) continue;
		const identity = effectiveIdentityColumn(overlay, mapping.label);
		if (identity !== null) keys.push([identity]);
	}
	for (const mapping of overlay.edges) {
		if (mapping.table === table) {
			keys.push([mapping.src_column, mapping.dst_column]);
		}
	}
	return keys;
}

function sameKeyValue(left: unknown, right: unknown): boolean {
	if (left === null || left === undefined) return false;
	if (right === null || right === undefined) return false;
	if (sameJsonValue(left, right)) return true;
	// An endpoint read from a node id is text even when the column is a number.
	return (
		typeof left !== "object" &&
		typeof right !== "object" &&
		String(left) === String(right)
	);
}

function shownChanges(
	props: Record<string, unknown>,
	fresh: Readonly<Record<string, unknown>>,
): Record<string, unknown> | null {
	const changes = Object.entries(fresh).filter(
		([key, value]) =>
			Object.hasOwn(props, key) && !sameJsonValue(props[key], value),
	);
	return changes.length > 0 ? Object.fromEntries(changes) : null;
}

/** The row values an edge reads, its endpoint columns included. */
function edgeRowValues(
	overlay: GraphOverlay,
	edge: SubgraphEdge,
	mapping: EdgeLabelMapping,
	nodesById: ReadonlyMap<string, SubgraphNode>,
): Record<string, unknown> {
	return {
		...edge.props,
		[mapping.src_column]: endpointIdentity(
			overlay,
			edge.source,
			mapping.src_label,
			nodesById.get(edge.source),
		),
		[mapping.dst_column]: endpointIdentity(
			overlay,
			edge.target,
			mapping.dst_label,
			nodesById.get(edge.target),
		),
	};
}

/**
 * Applies a saved row to every other loaded element that reads the same stored
 * row: another label mapped on the table, a foreign-key relationship whose
 * values live on the object, or a join table that is also mapped as an object.
 * Only values an element already shows are replaced; the id sets never change.
 */
export function patchSubgraphRowCopies(
	data: SubgraphResult | null,
	overlay: GraphOverlay | null,
	saved: SavedRow,
	edited: { nodeId?: string; edgeId?: string },
): SubgraphResult | null {
	if (!data || !overlay) return data;
	const keys = rowKeyColumns(overlay, saved.table);
	const readsSavedRow = (values: Record<string, unknown>) =>
		keys.some((columns) =>
			columns.every((column) =>
				sameKeyValue(values[column], saved.known[column]),
			),
		);
	const nodeTables = new Map(
		overlay.nodes.map((mapping) => [mapping.label, mapping.table]),
	);
	const edgeLabels = new Set(
		overlay.edges
			.filter((mapping) => mapping.table === saved.table)
			.map((mapping) => mapping.label),
	);
	const nodesById = new Map(data.nodes.map((node) => [node.id, node]));
	let changed = false;

	const nodes = data.nodes.map((node) => {
		if (
			node.id === edited.nodeId ||
			nodeTables.get(node.label) !== saved.table ||
			!readsSavedRow(node.props)
		) {
			return node;
		}
		const patch = shownChanges(node.props, saved.fresh);
		if (!patch) return node;
		changed = true;
		return patchNodeProps(node, patch, overlay);
	});

	const edges = data.edges.map((edge) => {
		if (edge.id === edited.edgeId || !edgeLabels.has(edge.label)) return edge;
		const mapping = resolveEdgeMapping(
			overlay,
			edge,
			nodesById.get(edge.source)?.label,
			nodesById.get(edge.target)?.label,
		);
		if (
			mapping?.table !== saved.table ||
			!readsSavedRow(edgeRowValues(overlay, edge, mapping, nodesById))
		) {
			return edge;
		}
		const patch = shownChanges(edge.props, saved.fresh);
		if (!patch) return edge;
		changed = true;
		return { ...edge, props: { ...edge.props, ...patch } };
	});

	return changed ? { ...data, nodes, edges } : data;
}

/** Swaps in a relationship's freshly saved properties, keeping the edge set. */
export function patchSubgraphEdge(
	data: SubgraphResult | null,
	edgeId: string,
	fresh: Record<string, unknown>,
): SubgraphResult | null {
	const index = data?.edges.findIndex((edge) => edge.id === edgeId) ?? -1;
	if (!data || index < 0) return data;
	const edges = [...data.edges];
	edges[index] = {
		...data.edges[index],
		props: { ...data.edges[index].props, ...fresh },
	};
	return { ...data, edges };
}

export function applyStyleToOverlay(
	overlay: GraphOverlay,
	label: string,
	type: "node" | "edge",
	style: LabelStyle,
): GraphOverlay {
	if (type === "node") {
		return {
			...overlay,
			nodes: overlay.nodes.map((node) =>
				node.label === label ? { ...node, style } : node,
			),
		};
	}
	return {
		...overlay,
		edges: overlay.edges.map((edge) =>
			edge.label === label ? { ...edge, style } : edge,
		),
	};
}

/** Derived from the label alone, so a label keeps its colour whatever else the data contains. */
function syntheticStyle(label: string): LabelStyle {
	return {
		color: SYNTHETIC_PALETTE[labelSeed(label) % SYNTHETIC_PALETTE.length],
		icon: "circle",
		size: { mode: "by-degree", min: 8, max: 20 },
	};
}

function toNodeMapping(
	label: string,
	styles?: Record<string, LabelStyle | undefined>,
): NodeLabelMapping {
	return {
		label,
		table: label,
		id_column: "id",
		property_columns: [],
		style: styles?.[label] ?? syntheticStyle(label),
	};
}

function toEdgeMapping(
	label: string,
	styles?: Record<string, LabelStyle | undefined>,
): EdgeLabelMapping {
	return {
		label,
		table: label,
		src_column: "source",
		dst_column: "target",
		src_label: "",
		dst_label: "",
		property_columns: [],
		style: styles?.[label] ?? syntheticStyle(label),
	};
}

/**
 * Builds a throwaway overlay from raw subgraph data so the ontology-backed
 * viewer (legend, inspectors, styling) also drives graphs that have no overlay
 * behind them. Labels missing from `labelStyles` get a stable generated colour.
 */
export function buildOverlayFromSubgraph(
	nodes: readonly SubgraphNode[],
	edges: readonly SubgraphEdge[],
	options?: {
		name?: string;
		description?: string;
		labelStyles?: Record<string, LabelStyle | undefined>;
	},
): GraphOverlay {
	const nodeLabels: string[] = [];
	const edgeLabels: string[] = [];
	const seenNodeLabels = new Set<string>();
	const seenEdgeLabels = new Set<string>();

	for (const node of nodes) {
		if (!node?.label || seenNodeLabels.has(node.label)) continue;
		seenNodeLabels.add(node.label);
		nodeLabels.push(node.label);
	}
	for (const edge of edges) {
		if (!edge?.label || seenEdgeLabels.has(edge.label)) continue;
		seenEdgeLabels.add(edge.label);
		edgeLabels.push(edge.label);
	}

	return {
		id: "inline",
		name: options?.name ?? "Graph",
		description: options?.description,
		nodes: nodeLabels.map((label) =>
			toNodeMapping(label, options?.labelStyles),
		),
		edges: edgeLabels.map((label) =>
			toEdgeMapping(label, options?.labelStyles),
		),
		object_views: [],
		actions: [],
		exposed: false,
		bindings_enabled: false,
		default_limit: nodes.length,
		created_at: "",
		updated_at: "",
	};
}

import type {
	EdgeLabelMapping,
	GraphOverlay,
	LabelStyle,
	NodeLabelMapping,
	SubgraphEdge,
	SubgraphNode,
	SubgraphResult,
} from "../../../state/backend-state/graph-state";

export const DEFAULT_LABEL_STYLE: LabelStyle = {
	color: "#6b7280",
	icon: "circle",
	size: { mode: "fixed", value: 6 },
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
		size: { mode: "fixed", value: 6 },
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

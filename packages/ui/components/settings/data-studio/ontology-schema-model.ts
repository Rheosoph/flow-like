import type {
	EdgeLabelMapping,
	NodeLabelMapping,
} from "../../../state/backend-state/graph-state";

export const SCHEMA_NODE_WIDTH = 232;
export const SCHEMA_HEADER_HEIGHT = 48;
export const SCHEMA_ROW_HEIGHT = 22;
// Outer border (2) and the row list's top border plus vertical padding (9).
const SCHEMA_FRAME_HEIGHT = 2;
const SCHEMA_ROWS_CHROME_HEIGHT = 9;
const MAX_VISIBLE_ROWS = 6;
const PARALLEL_SPACING = 34;

export type SchemaObjectKind = "object" | "external" | "missing";
export type SchemaRowRole = "id" | "link" | "property";

export interface SchemaRow {
	name: string;
	dataType?: string;
	role: SchemaRowRole;
}

export interface SchemaObject {
	id: string;
	kind: SchemaObjectKind;
	label: string;
	/** Source table for objects, the owning ontology's name for external targets. */
	subtitle?: string;
	color?: string;
	icon?: string;
	rows: SchemaRow[];
	hiddenRows: number;
	height: number;
	mapping?: NodeLabelMapping;
}

export interface SchemaRelationship {
	id: string;
	index: number;
	source: string;
	target: string;
	label: string;
	join: string;
	containment: boolean;
	external: boolean;
	/** Perpendicular bend in px, signed so parallel edges fan out on both sides. */
	offset: number;
	/** Nesting depth among self-references on the same object. */
	loop: number;
}

export interface SchemaModel {
	objects: SchemaObject[];
	relationships: SchemaRelationship[];
}

export interface SchemaRect {
	x: number;
	y: number;
	width: number;
	height: number;
}

export interface SchemaEdgeGeometry {
	path: string;
	labelX: number;
	labelY: number;
}

/** `local:{overlayId}` / `remote:{importId}` for children that live in another ontology. */
export function externalTargetKey(
	edge: Pick<EdgeLabelMapping, "dst_ontology" | "dst_binding_id">,
): string | undefined {
	if (edge.dst_ontology) return `local:${edge.dst_ontology}`;
	if (edge.dst_binding_id) return `remote:${edge.dst_binding_id}`;
	return undefined;
}

function schemaObjectHeight(rowCount: number): number {
	if (rowCount === 0) return SCHEMA_FRAME_HEIGHT + SCHEMA_HEADER_HEIGHT;
	return (
		SCHEMA_FRAME_HEIGHT +
		SCHEMA_HEADER_HEIGHT +
		SCHEMA_ROWS_CHROME_HEIGHT +
		rowCount * SCHEMA_ROW_HEIGHT
	);
}

/** Identity first, then columns relationships join on, then plain properties. */
function objectRows(
	node: NodeLabelMapping,
	edges: readonly EdgeLabelMapping[],
): { rows: SchemaRow[]; hiddenRows: number } {
	const typeOf = new Map(
		node.property_columns.map((column) => [column.name, column.data_type]),
	);
	const linkColumns: string[] = [];
	for (const edge of edges) {
		if (edge.table !== node.table) continue;
		for (const column of [edge.src_column, edge.dst_column]) {
			if (column && column !== node.id_column && !linkColumns.includes(column)) {
				linkColumns.push(column);
			}
		}
	}
	const linkSet = new Set(linkColumns);
	const ranked: SchemaRow[] = [
		...linkColumns.map((name) => ({
			name,
			dataType: typeOf.get(name),
			role: "link" as const,
		})),
		...node.property_columns
			.filter(
				(column) => column.name !== node.id_column && !linkSet.has(column.name),
			)
			.map((column) => ({
				name: column.name,
				dataType: column.data_type,
				role: "property" as const,
			})),
	];
	const visible = ranked.slice(0, MAX_VISIBLE_ROWS - 1);
	return {
		rows: [
			{ name: node.id_column, dataType: typeOf.get(node.id_column), role: "id" },
			...visible,
		],
		hiddenRows: ranked.length - visible.length,
	};
}

function ghostObject(
	id: string,
	kind: Exclude<SchemaObjectKind, "object">,
	label: string,
	subtitle?: string,
): SchemaObject {
	return {
		id,
		kind,
		label,
		subtitle,
		rows: [],
		hiddenRows: 0,
		height: schemaObjectHeight(0),
	};
}

function labelKey(label: string): string {
	return label.trim().toLowerCase();
}

/**
 * Turns an ontology's object and relationship mappings into diagram nodes and
 * edges. Labels share one case-insensitive namespace, so endpoints resolve by
 * label; a child that lives in another ontology, or a label that no object in
 * this one carries, becomes a ghost node instead of a dangling edge.
 */
export function buildSchemaModel(
	nodes: readonly NodeLabelMapping[],
	edges: readonly EdgeLabelMapping[],
	externalTargets: ReadonlyMap<string, string> = new Map(),
): SchemaModel {
	const objects: SchemaObject[] = [];
	const idByLabel = new Map<string, string>();
	for (const node of nodes) {
		const key = labelKey(node.label);
		if (idByLabel.has(key)) continue;
		const id = `object:${key}`;
		idByLabel.set(key, id);
		const { rows, hiddenRows } = objectRows(node, edges);
		objects.push({
			id,
			kind: "object",
			label: node.label,
			subtitle: node.table,
			color: node.style.color,
			icon: node.style.icon,
			rows,
			hiddenRows,
			height: schemaObjectHeight(rows.length + (hiddenRows > 0 ? 1 : 0)),
			mapping: node,
		});
	}

	const ghosts = new Map<string, SchemaObject>();
	const resolveLocal = (label: string): string => {
		const key = labelKey(label);
		const id = idByLabel.get(key);
		if (id) return id;
		const ghostId = `missing:${key}`;
		if (!ghosts.has(ghostId)) {
			ghosts.set(ghostId, ghostObject(ghostId, "missing", label));
		}
		return ghostId;
	};
	const resolveTarget = (edge: EdgeLabelMapping): string => {
		const external = externalTargetKey(edge);
		if (!external) return resolveLocal(edge.dst_label);
		const ghostId = `external:${external}:${labelKey(edge.dst_label)}`;
		if (!ghosts.has(ghostId)) {
			ghosts.set(
				ghostId,
				ghostObject(
					ghostId,
					"external",
					edge.dst_label,
					externalTargets.get(external),
				),
			);
		}
		return ghostId;
	};

	const endpoints = edges.map((edge) => ({
		source: resolveLocal(edge.src_label),
		target: resolveTarget(edge),
	}));

	const groupOf = ({ source, target }: { source: string; target: string }) =>
		source === target
			? `loop:${source}`
			: [source, target].sort().join("\u0000");
	const groupSize = new Map<string, number>();
	for (const endpoint of endpoints) {
		const group = groupOf(endpoint);
		groupSize.set(group, (groupSize.get(group) ?? 0) + 1);
	}
	const groupSeen = new Map<string, number>();

	const relationships = edges.map((edge, index): SchemaRelationship => {
		const { source, target } = endpoints[index];
		const group = groupOf(endpoints[index]);
		const position = groupSeen.get(group) ?? 0;
		groupSeen.set(group, position + 1);
		const size = groupSize.get(group) ?? 1;
		const isLoop = source === target;
		// Offsets are measured against the id-sorted direction, so A→B and B→A
		// between the same pair still land on opposite sides.
		const direction = source < target ? 1 : -1;
		return {
			id: `relationship:${index}`,
			index,
			source,
			target,
			label: edge.label,
			join: `${edge.table}.${edge.src_column} → ${edge.table}.${edge.dst_column}`,
			containment: Boolean(edge.containment),
			external: externalTargetKey(edge) !== undefined,
			offset: isLoop
				? 0
				: (position - (size - 1) / 2) * PARALLEL_SPACING * direction,
			loop: isLoop ? position : 0,
		};
	});

	return { objects: [...objects, ...ghosts.values()], relationships };
}

function rectCenter(rect: SchemaRect): { x: number; y: number } {
	return { x: rect.x + rect.width / 2, y: rect.y + rect.height / 2 };
}

/** Where the ray from the rect's centre towards `towards` leaves the rect. */
export function boundaryPoint(
	rect: SchemaRect,
	towards: { x: number; y: number },
): { x: number; y: number } {
	const center = rectCenter(rect);
	const dx = towards.x - center.x;
	const dy = towards.y - center.y;
	const halfWidth = rect.width / 2;
	const halfHeight = rect.height / 2;
	if ((dx === 0 && dy === 0) || halfWidth <= 0 || halfHeight <= 0) {
		return center;
	}
	const scale = 1 / Math.max(Math.abs(dx) / halfWidth, Math.abs(dy) / halfHeight);
	return { x: center.x + dx * scale, y: center.y + dy * scale };
}

/** A quadratic curve between the two rect borders, bent by `offset` px at its apex. */
export function curveGeometry(
	source: SchemaRect,
	target: SchemaRect,
	offset: number,
): SchemaEdgeGeometry {
	const from = rectCenter(source);
	const to = rectCenter(target);
	const dx = to.x - from.x;
	const dy = to.y - from.y;
	const length = Math.hypot(dx, dy) || 1;
	// A quadratic's apex sits halfway to its control point.
	const control = {
		x: (from.x + to.x) / 2 - (dy / length) * offset * 2,
		y: (from.y + to.y) / 2 + (dx / length) * offset * 2,
	};
	const start = boundaryPoint(source, offset === 0 ? to : control);
	const end = boundaryPoint(target, offset === 0 ? from : control);
	return {
		path: `M ${start.x} ${start.y} Q ${control.x} ${control.y} ${end.x} ${end.y}`,
		labelX: 0.25 * start.x + 0.5 * control.x + 0.25 * end.x,
		labelY: 0.25 * start.y + 0.5 * control.y + 0.25 * end.y,
	};
}

/** A self-reference drawn as a loop over the rect's top-right corner. */
export function loopGeometry(
	rect: SchemaRect,
	loop: number,
): SchemaEdgeGeometry {
	const reach = 48 + loop * 20;
	const startX = rect.x + rect.width - 40 - loop * 12;
	const startY = rect.y;
	const endX = rect.x + rect.width;
	const endY = rect.y + 24 + loop * 8;
	return {
		path: `M ${startX} ${startY} C ${startX} ${startY - reach} ${endX + reach} ${endY} ${endX} ${endY}`,
		labelX: 0.5 * startX + 0.375 * (endX + reach) + 0.125 * endX,
		labelY: 0.125 * startY + 0.375 * (startY - reach) + 0.5 * endY,
	};
}

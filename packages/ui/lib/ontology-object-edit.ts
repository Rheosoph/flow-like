import {
	type ArrowSchemaJSON,
	type LanceFieldKind,
	arrowToLanceSchema,
	resolveTemporalCell,
} from "../components/ui/lance-viewer";
import type { TemporalCell } from "../components/ui/temporal-value-editor";
import type {
	EdgeLabelMapping,
	GraphOverlay,
	NodeLabelMapping,
	SubgraphEdge,
	SubgraphNode,
	UpdateOntologyObjectPayload,
	UpdateOntologyRelationshipPayload,
} from "../state/backend-state/graph-state";

export type OntologyIdentityValue = string | number | boolean;

export function objectTypeKey(m: {
	id?: string;
	api_name?: string;
	label: string;
}): string {
	return m.id ?? m.api_name ?? m.label;
}

/**
 * The column that identifies `label` across the graph, mirroring the server's
 * `effective_node_id_column_for_mappings`: an edge's node-column override wins
 * over the node's own id column, and overrides that disagree identify nothing.
 */
export function effectiveIdentityColumn(
	overlay: GraphOverlay,
	label: string,
): string | null {
	const node = overlay.nodes.find((mapping) => mapping.label === label);
	if (!node) return null;
	const overrides = new Set(
		overlay.edges.flatMap((edge) => identityOverrides(edge, label)),
	);
	if (overrides.size > 1) return null;
	const [override] = overrides;
	return override ?? node.id_column;
}

function identityOverrides(edge: EdgeLabelMapping, label: string): string[] {
	const columns: string[] = [];
	if (edge.src_label === label && typeof edge.src_node_column === "string") {
		columns.push(edge.src_node_column);
	}
	if (edge.dst_label === label && typeof edge.dst_node_column === "string") {
		columns.push(edge.dst_node_column);
	}
	return columns;
}

export type ColumnLock = "identity" | "relationship";

export function lockedObjectColumns(
	overlay: GraphOverlay,
	mapping: NodeLabelMapping,
): ReadonlyMap<string, ColumnLock> {
	const locks = new Map<string, ColumnLock>();
	const identity = effectiveIdentityColumn(overlay, mapping.label);
	if (identity) locks.set(identity, "identity");
	locks.set(mapping.id_column, "identity");
	for (const edge of overlay.edges) {
		if (edge.table !== mapping.table) continue;
		for (const column of [edge.src_column, edge.dst_column]) {
			if (!locks.has(column)) locks.set(column, "relationship");
		}
	}
	return locks;
}

/**
 * The identity column of the object a relationship is stored on when it is a
 * foreign key on that object's own table; null for join tables, whose rows are
 * relationships in their own right. Mirrors the server's
 * `foreign_key_row_identity`.
 */
export function foreignKeyRowIdentity(
	overlay: GraphOverlay,
	table: string,
	src: string,
	dst: string,
): string | null {
	for (const node of overlay.nodes) {
		if (node.table !== table) continue;
		const identity = effectiveIdentityColumn(overlay, node.label);
		if (identity !== null && (identity === src || identity === dst)) {
			return identity;
		}
	}
	return null;
}

export function resolveEdgeMapping(
	overlay: GraphOverlay,
	edge: SubgraphEdge,
	sourceLabel?: string,
	targetLabel?: string,
): EdgeLabelMapping | undefined {
	const candidates = overlay.edges.filter(
		(mapping) =>
			mapping.label === edge.label &&
			(sourceLabel === undefined || mapping.src_label === sourceLabel) &&
			(targetLabel === undefined || mapping.dst_label === targetLabel),
	);
	if (candidates.length <= 1) return candidates[0];
	const byEndpoints = candidates.filter(
		(mapping) =>
			edge.source.startsWith(`${mapping.src_label}:`) &&
			edge.target.startsWith(`${mapping.dst_label}:`),
	);
	return byEndpoints.length === 1 ? byEndpoints[0] : undefined;
}

export function lockedRelationshipColumns(
	overlay: GraphOverlay,
	mapping: EdgeLabelMapping,
): ReadonlyMap<string, ColumnLock> {
	const locks = new Map<string, ColumnLock>([
		[mapping.src_column, "relationship"],
		[mapping.dst_column, "relationship"],
	]);
	for (const node of overlay.nodes) {
		if (node.table !== mapping.table) continue;
		for (const [column, lock] of lockedObjectColumns(overlay, node)) {
			if (!locks.has(column)) locks.set(column, lock);
		}
	}
	return locks;
}

function isUnsafeInteger(value: unknown): boolean {
	return (
		typeof value === "number" &&
		Number.isInteger(value) &&
		!Number.isSafeInteger(value)
	);
}

function isIdentityValue(value: unknown): value is OntologyIdentityValue {
	return (
		typeof value === "string" ||
		typeof value === "number" ||
		typeof value === "boolean"
	);
}

export type EditableIdentity =
	| {
			ok: true;
			mapping: NodeLabelMapping;
			identityColumn: string;
			id: OntologyIdentityValue;
	  }
	| {
			ok: false;
			reason:
				| "unknownType"
				| "crossOntology"
				| "identityConflict"
				| "identityMissing"
				| "identityUnsafe";
	  };

export function resolveObjectIdentity(
	overlay: GraphOverlay,
	label: string,
	row: Record<string, unknown>,
): EditableIdentity {
	const mapping = overlay.nodes.find((node) => node.label === label);
	if (!mapping) {
		const crossOntology = overlay.edges.some(
			(edge) =>
				edge.dst_label === label &&
				Boolean(edge.dst_ontology || edge.dst_binding_id),
		);
		return {
			ok: false,
			reason: crossOntology ? "crossOntology" : "unknownType",
		};
	}
	const identityColumn = effectiveIdentityColumn(overlay, label);
	if (identityColumn === null) return { ok: false, reason: "identityConflict" };
	const id = row[identityColumn];
	if (!isIdentityValue(id)) return { ok: false, reason: "identityMissing" };
	if (isUnsafeInteger(id)) return { ok: false, reason: "identityUnsafe" };
	return { ok: true, mapping, identityColumn, id };
}

export type EditableRelationship =
	| {
			ok: true;
			mapping: EdgeLabelMapping;
			source: OntologyIdentityValue;
			target: OntologyIdentityValue;
	  }
	| {
			ok: false;
			reason: "foreignKey";
			ownerLabel: string;
			ownerSide: "source" | "target";
	  }
	| { ok: false; reason: "unknownType" | "identityMissing" | "identityUnsafe" };

/**
 * The identity value a relationship endpoint stores: the loaded node's typed
 * identity when it has one, else the text after the label in its id.
 */
export function endpointIdentity(
	overlay: GraphOverlay,
	nodeId: string,
	mappingLabel: string,
	node?: SubgraphNode,
): OntologyIdentityValue | undefined {
	if (node) {
		const column = effectiveIdentityColumn(overlay, node.label);
		const value = column === null ? undefined : node.props[column];
		if (isIdentityValue(value)) return value;
	}
	const prefix = `${node?.label ?? mappingLabel}:`;
	return nodeId.startsWith(prefix) ? nodeId.slice(prefix.length) : undefined;
}

function foreignKeyOwner(
	overlay: GraphOverlay,
	mapping: EdgeLabelMapping,
): Extract<EditableRelationship, { reason: "foreignKey" }> | null {
	const foreignKey = foreignKeyRowIdentity(
		overlay,
		mapping.table,
		mapping.src_column,
		mapping.dst_column,
	);
	if (foreignKey === null) return null;
	const ownerSide = foreignKey === mapping.src_column ? "source" : "target";
	return {
		ok: false,
		reason: "foreignKey",
		ownerSide,
		ownerLabel: ownerSide === "source" ? mapping.src_label : mapping.dst_label,
	};
}

export function resolveRelationshipIdentity(
	overlay: GraphOverlay,
	edge: SubgraphEdge,
	sourceNode?: SubgraphNode,
	targetNode?: SubgraphNode,
): EditableRelationship {
	const mapping = resolveEdgeMapping(
		overlay,
		edge,
		sourceNode?.label,
		targetNode?.label,
	);
	if (!mapping) return { ok: false, reason: "unknownType" };
	const owner = foreignKeyOwner(overlay, mapping);
	if (owner) return owner;
	const source = endpointIdentity(
		overlay,
		edge.source,
		mapping.src_label,
		sourceNode,
	);
	const target = endpointIdentity(
		overlay,
		edge.target,
		mapping.dst_label,
		targetNode,
	);
	if (source === undefined || target === undefined) {
		return { ok: false, reason: "identityMissing" };
	}
	if (isUnsafeInteger(source) || isUnsafeInteger(target)) {
		return { ok: false, reason: "identityUnsafe" };
	}
	return { ok: true, mapping, source, target };
}

export interface ObjectEditField {
	name: string;
	kind: LanceFieldKind;
	/** Whole-number storage: an edit must not carry a fraction. */
	integer: boolean;
	nullable: boolean;
	/** Set for date-kind columns: the unit and shape an edit writes back. */
	temporal: TemporalCell | null;
}

const INTEGER_ARROW_TYPES = new Set([
	"Int8",
	"Int16",
	"Int32",
	"Int64",
	"UInt8",
	"UInt16",
	"UInt32",
	"UInt64",
]);

function isIntegerArrowType(dataType: unknown): boolean {
	if (typeof dataType === "string") return INTEGER_ARROW_TYPES.has(dataType);
	if (!dataType || typeof dataType !== "object") return false;
	const type = dataType as Record<string, unknown>;
	if (Array.isArray(type.Dictionary))
		return isIntegerArrowType(type.Dictionary[1]);
	return (
		type.Time32 !== undefined ||
		type.Time64 !== undefined ||
		type.Duration !== undefined
	);
}

export function objectEditFields(
	arrow: ArrowSchemaJSON,
): ReadonlyMap<string, ObjectEditField> {
	const raw = arrow?.fields ?? [];
	const fields = new Map<string, ObjectEditField>();
	arrowToLanceSchema(arrow).fields.forEach((field, index) => {
		fields.set(field.name, {
			name: field.name,
			kind: field.kind,
			integer:
				field.kind === "number" && isIntegerArrowType(raw[index]?.data_type),
			nullable: field.nullable !== false,
			temporal:
				field.kind === "date" ? resolveTemporalCell(field, undefined) : null,
		});
	});
	return fields;
}

export type PropertyEditorKind =
	| "text"
	| "integer"
	| "number"
	| "boolean"
	| "temporal";

export type PropertyLockReason =
	| ColumnLock
	| "kind"
	| "unknownType"
	| "unsafeInteger";

export type EditableProperty = {
	editor: PropertyEditorKind;
	field: ObjectEditField;
};

export type PropertyEditability =
	| EditableProperty
	| { locked: PropertyLockReason; kind?: string };

const UNEDITABLE_KINDS = new Set<LanceFieldKind>([
	"geometry",
	"binary",
	"vector",
	"array",
	"object",
]);

export function isEditableProperty(
	editability: PropertyEditability,
): editability is EditableProperty {
	return "editor" in editability;
}

export function propertyEditability(
	name: string,
	value: unknown,
	field: ObjectEditField | undefined,
	locked: ReadonlyMap<string, ColumnLock>,
): PropertyEditability {
	const lock = locked.get(name);
	if (lock) return { locked: lock };
	if (!field) return { locked: "unknownType" };
	if (UNEDITABLE_KINDS.has(field.kind)) {
		return { locked: "kind", kind: field.kind };
	}
	if (isUnsafeInteger(value)) return { locked: "unsafeInteger" };
	if (field.kind === "date") return temporalEditability(field, value);
	const editor = scalarEditor(field, value);
	return editor ? { editor, field } : { locked: "unknownType" };
}

/** Writes back in the stored shape: an ISO string stays one, a number stays one. */
function temporalEditability(
	field: ObjectEditField,
	value: unknown,
): PropertyEditability {
	if (!field.temporal) return { locked: "unknownType" };
	return {
		editor: "temporal",
		field: {
			...field,
			temporal: {
				...field.temporal,
				wire: typeof value === "string" ? "string" : "number",
			},
		},
	};
}

function scalarEditor(
	field: ObjectEditField,
	value: unknown,
): PropertyEditorKind | null {
	switch (field.kind) {
		case "boolean":
			return "boolean";
		case "number":
			return field.integer ? "integer" : "number";
		case "string":
			return "text";
		default:
			return typeof value === "string" ? "text" : null;
	}
}

export interface PropertyDraft {
	text: string;
	isNull: boolean;
}

export type PropertyDraftError =
	| "number"
	| "integer"
	| "unsafeInteger"
	| "boolean"
	| "temporal";

type EditorInput = PropertyEditorKind | { readonly editor: PropertyEditorKind };

function editorOf(e: EditorInput): PropertyEditorKind {
	return typeof e === "string" ? e : e.editor;
}

export function draftFromValue(e: EditorInput, value: unknown): PropertyDraft {
	if (value === null || value === undefined) return { text: "", isNull: true };
	if (editorOf(e) === "temporal") {
		return { text: JSON.stringify(value), isNull: false };
	}
	if (typeof value === "object") {
		return { text: JSON.stringify(value), isNull: false };
	}
	return { text: String(value), isNull: false };
}

const DECIMAL_PATTERN = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/;
const INTEGER_PATTERN = /^-?\d+$/;

export function parsePropertyDraft(
	e: EditorInput,
	draft: PropertyDraft,
): { ok: true; value: unknown } | { ok: false; error: PropertyDraftError } {
	if (draft.isNull) return { ok: true, value: null };
	const trimmed = draft.text.trim();
	switch (editorOf(e)) {
		case "text":
			return { ok: true, value: draft.text };
		case "integer": {
			if (!INTEGER_PATTERN.test(trimmed))
				return { ok: false, error: "integer" };
			const value = Number(trimmed);
			return Number.isSafeInteger(value)
				? { ok: true, value }
				: { ok: false, error: "unsafeInteger" };
		}
		case "number": {
			const value = Number(trimmed);
			return DECIMAL_PATTERN.test(trimmed) && Number.isFinite(value)
				? { ok: true, value }
				: { ok: false, error: "number" };
		}
		case "boolean":
			if (trimmed === "true") return { ok: true, value: true };
			if (trimmed === "false") return { ok: true, value: false };
			return { ok: false, error: "boolean" };
		case "temporal": {
			let value: unknown;
			try {
				value = JSON.parse(draft.text);
			} catch {
				return { ok: false, error: "temporal" };
			}
			return value === null ||
				typeof value === "string" ||
				(typeof value === "number" && Number.isFinite(value))
				? { ok: true, value }
				: { ok: false, error: "temporal" };
		}
	}
}

function sameValue(left: unknown, right: unknown): boolean {
	return JSON.stringify(left ?? null) === JSON.stringify(right ?? null);
}

export function changedProperties(
	baseline: Record<string, unknown>,
	next: Record<string, unknown>,
): Record<string, unknown> {
	return Object.fromEntries(
		Object.entries(next).filter(
			([key, value]) => !sameValue(baseline[key], value),
		),
	);
}

function expectedFor(
	baseline: Record<string, unknown>,
	updates: Record<string, unknown>,
): Record<string, unknown> {
	return Object.fromEntries(
		Object.keys(updates).map((key) => [key, baseline[key] ?? null]),
	);
}

export function buildObjectUpdate(
	identity: Extract<EditableIdentity, { ok: true }>,
	baseline: Record<string, unknown>,
	updates: Record<string, unknown>,
): UpdateOntologyObjectPayload {
	return {
		object_type: objectTypeKey(identity.mapping),
		id: identity.id,
		updates,
		expected: expectedFor(baseline, updates),
	};
}

export function buildRelationshipUpdate(
	relationship: Extract<EditableRelationship, { ok: true }>,
	baselineProps: Record<string, unknown>,
	updates: Record<string, unknown>,
): UpdateOntologyRelationshipPayload {
	return {
		relationship_type: objectTypeKey(relationship.mapping),
		source: relationship.source,
		target: relationship.target,
		updates,
		expected: expectedFor(baselineProps, updates),
	};
}

/** The row changed after it was loaded; `current` is what the server holds now. */
export class StaleObjectError extends Error {
	readonly current: Record<string, unknown>;

	constructor(current: Record<string, unknown>) {
		super("The object changed after it was loaded");
		this.name = "StaleObjectError";
		this.current = current;
	}
}

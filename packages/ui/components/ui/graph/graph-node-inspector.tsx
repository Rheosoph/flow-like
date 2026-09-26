"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	ChevronDown,
	ChevronsDownUp,
	Copy,
	Crosshair,
	Expand,
	Eye,
	EyeOff,
	Filter,
	ListTree,
	Route,
	SlidersHorizontal,
	Workflow,
	X,
} from "lucide-react";
import type React from "react";
import {
	createContext,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { inferTemporalValue } from "../../../lib/date";
import { namesGeometryKind } from "../../../lib/geometry";
import { isGeometryMetadata } from "../../../lib/geometry-columns";
import {
	type ColumnLock,
	type EditableIdentity,
	type ObjectEditField,
	type PropertyEditability,
	isEditableProperty,
	lockedObjectColumns,
	propertyEditability,
	resolveObjectIdentity,
} from "../../../lib/ontology-object-edit";
import { resolveStorageFile } from "../../../lib/storage-file";
import { looksLikeUserColumnName } from "../../../lib/user-display";
import type {
	GraphOverlay,
	NodeLabelMapping,
	OntologyActionDefinition,
	PropertyColumn,
	SubgraphNode,
} from "../../../state/backend-state/graph-state";
import { accountIdFromValue } from "../../../state/backend-state/user-state";
import { Badge } from "../badge";
import { BinaryCellPreview, BinaryValueDetail } from "../binary-value-cell";
import { Button } from "../button";
import { GeometryCell } from "../geometry-cell";
import { Popover, PopoverContent, PopoverTrigger } from "../popover";
import { RelativeTime } from "../relative-time";
import { ScrollArea } from "../scroll-area";
import { StorageFileCell } from "../storage-file-cell";
import { UserInlineTag } from "../user-identity";
import { nodeCaptionAccountId } from "./graph-user-caption";
import { getGraphIcon } from "./icons";
import {
	InlinePropertyEditor,
	PropertyEditButton,
	PropertyLockHint,
} from "./ontology-property-editor";

export interface ConnectionInfo {
	label: string;
	direction: "outgoing" | "incoming";
	targetCaption: string;
	targetAccountId?: string | null;
	targetId: string;
}

export interface GraphNodeInspectorProps {
	node: SubgraphNode | null;
	overlay?: GraphOverlay;
	connections?: ConnectionInfo[];
	onClose: () => void;
	onExpand?: (depth: number) => void;
	/** Opens the expansion guard, where relationships and a ceiling are chosen. */
	onGuidedExpand?: () => void;
	/** Restricts the canvas to this object's neighborhood; null leaves focus. */
	onFocus?: (depth: number | null) => void;
	focused?: boolean;
	hasChildren?: boolean;
	childrenExpanded?: boolean;
	onExpandChildren?: () => void;
	onCollapseChildren?: () => void;
	onConnectionClick?: (nodeId: string) => void;
	onFindPath?: (node: SubgraphNode) => void;
	onRunAction?: (action: OntologyActionDefinition, node: SubgraphNode) => void;
	/** Column types of the node's backing table; undefined while they load. */
	editFields?: ReadonlyMap<string, ObjectEditField>;
	/**
	 * Saves edited property values. Omitted, the inspector is read-only.
	 * Rejects with StaleObjectError when the stored row moved on.
	 */
	onUpdateProperties?: UpdateElementProperties;
}

function objectTypeMatches(
	mapping: NodeLabelMapping,
	objectType: string,
): boolean {
	return (
		objectType === mapping.id ||
		objectType === mapping.api_name ||
		objectType === mapping.label
	);
}

export type ValueKind =
	| "geometry"
	| "binary"
	| "file"
	| "string"
	| "number"
	| "boolean"
	| "date"
	| "user"
	| "vector"
	| "array"
	| "object"
	| "unknown";

export {
	inferValueKind,
	declaredTypes,
	PropertyValue,
	PropertyRow,
	FieldFilter,
	CopyButton,
};

/**
 * The app whose storage the properties below it live in. Only the app that
 * owns the objects may provide it: a bare stored path resolves into the given
 * app's root, so a remote ontology's paths would open the wrong file.
 */
const PropertyStorageAppContext = createContext<string | undefined>(undefined);
export const PropertyStorageScope = PropertyStorageAppContext.Provider;
export const usePropertyStorageAppId = () =>
	useContext(PropertyStorageAppContext);

export interface PropertyValueContext {
	metadata?: Record<string, string>;
	/** The column type the ontology declares; most properties arrive without one. */
	typeName?: string;
	appId?: string;
}

/** Declared column types by property name, for the properties a mapping lists. */
function declaredTypes(
	columns?: readonly PropertyColumn[],
): ReadonlyMap<string, string> {
	return new Map(
		(columns ?? []).map((column) => [column.name, column.data_type]),
	);
}

/**
 * `propKey` is what makes an epoch integer readable: an ontology property is
 * untyped by the time it reaches here, so a `created_at` holding 1786353300000
 * is indistinguishable from a quantity without its name.
 */
function inferValueKind(
	value: unknown,
	propKey?: string,
	{ metadata, typeName, appId }: PropertyValueContext = {},
): { kind: ValueKind; dims?: number } {
	if (value === null || value === undefined) return { kind: "unknown" };
	if (isGeometryMetadata(metadata)) return { kind: "geometry" };
	if (typeName && /binary/i.test(typeName)) {
		// Storage hands a geoarrow column over as GeoJSON, and a declared type
		// string keeps only its `Binary` storage — plain bytes arrive as octets.
		if (namesGeometryKind(value)) return { kind: "geometry" };
		if (Array.isArray(value)) return { kind: "binary" };
	}
	if (typeof value === "boolean") return { kind: "boolean" };
	if (typeof value === "number" || typeof value === "bigint") {
		if (propKey && inferTemporalValue(propKey, value)) return { kind: "date" };
		return { kind: "number" };
	}
	if (Array.isArray(value)) {
		if (
			value.length > 0 &&
			value.every(
				(v) =>
					typeof v === "number" ||
					(typeof v === "string" && Number.isFinite(Number(v))),
			)
		) {
			return { kind: "vector", dims: value.length };
		}
		return { kind: "array" };
	}
	if (typeof value === "object") return { kind: "object" };
	if (typeof value === "string") {
		if (
			/^\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:?\d{2})?)?$/.test(
				value,
			)
		) {
			return { kind: "date" };
		}
		if (propKey && inferTemporalValue(propKey, value)) return { kind: "date" };
		// A property that names a person and holds an account id is that person.
		if (
			propKey &&
			looksLikeUserColumnName(propKey) &&
			accountIdFromValue(value)
		)
			return { kind: "user" };
		if (propKey && resolveStorageFile(propKey, value, appId))
			return { kind: "file" };
		return { kind: "string" };
	}
	return { kind: "unknown" };
}

function ensureNumericArray(v: unknown): number[] {
	if (Array.isArray(v)) return v.map(Number).filter((n) => Number.isFinite(n));
	return [];
}

const Sparkline: React.FC<{
	data: number[];
	width?: number;
	height?: number;
}> = ({ data, width = 120, height = 28 }) => {
	const ref = useRef<HTMLCanvasElement | null>(null);

	useEffect(() => {
		const canvas = ref.current;
		if (!canvas) return;

		const dpr = window.devicePixelRatio || 1;
		canvas.width = width * dpr;
		canvas.height = height * dpr;
		canvas.style.width = `${width}px`;
		canvas.style.height = `${height}px`;

		const ctx = canvas.getContext("2d");
		if (!ctx) return;

		ctx.scale(dpr, dpr);
		ctx.clearRect(0, 0, width, height);
		ctx.lineWidth = 1;
		ctx.beginPath();

		if (data.length === 0) return;

		const min = Math.min(...data);
		const max = Math.max(...data);
		const range = max - min || 1;

		for (let i = 0; i < data.length; i++) {
			const x = (i / Math.max(1, data.length - 1)) * (width - 2) + 1;
			const y = height - 1 - ((data[i] - min) / range) * (height - 2);
			if (i === 0) ctx.moveTo(x, y);
			else ctx.lineTo(x, y);
		}

		ctx.strokeStyle = getComputedStyle(
			document.documentElement,
		).getPropertyValue("--primary");
		ctx.stroke();
	}, [data, width, height]);

	return <canvas ref={ref} aria-label="sparkline" />;
};

function CopyButton({ text }: { text: string }) {
	const { t } = useTranslation("common");
	const [copied, setCopied] = useState(false);
	const handleCopy = useCallback(() => {
		navigator.clipboard.writeText(text);
		setCopied(true);
		setTimeout(() => setCopied(false), 1500);
	}, [text]);

	return (
		<button
			type="button"
			onClick={handleCopy}
			className="shrink-0 rounded p-1 text-muted-foreground opacity-60 transition-opacity hover:bg-accent hover:opacity-100 focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring group-hover:opacity-100"
			aria-label={t("copyValue", "Copy value")}
			title={t("copyValue", "Copy value")}
		>
			{copied ? (
				<Check className="h-3 w-3 text-green-500" />
			) : (
				<Copy className="h-3 w-3 text-muted-foreground" />
			)}
		</button>
	);
}

function PropertyValue({
	value,
	propKey,
	metadata,
	typeName,
	compact = false,
}: {
	value: unknown;
	propKey: string;
	metadata?: Record<string, string>;
	typeName?: string;
	compact?: boolean;
}) {
	const { t } = useTranslation("common");
	const appId = usePropertyStorageAppId();
	const { kind, dims } = inferValueKind(value, propKey, {
		metadata,
		typeName,
		appId,
	});

	if (kind === "geometry")
		return (
			<GeometryCell
				value={value}
				metadata={metadata}
				variant={compact ? "compact" : "card"}
			/>
		);
	if (kind === "binary")
		return compact ? (
			<BinaryCellPreview value={value} />
		) : (
			<BinaryValueDetail value={value} />
		);
	const file =
		kind === "file" ? resolveStorageFile(propKey, value, appId) : null;
	if (file && appId)
		return (
			<div className="group flex min-w-0 items-center justify-between gap-2">
				<StorageFileCell appId={appId} file={file} className="-ml-2 min-w-0" />
				<CopyButton text={String(value)} />
			</div>
		);

	const display =
		typeof value === "object"
			? JSON.stringify(value, null, 2)
			: String(value ?? "—");

	switch (kind) {
		case "boolean":
			return (
				<div className="group flex min-w-0 items-center justify-between gap-2">
					<span className="inline-flex items-center gap-1.5 rounded-md bg-muted px-2 py-1 text-xs font-medium">
						{value ? (
							<Check className="h-3.5 w-3.5 text-emerald-600 dark:text-emerald-400" />
						) : (
							<X className="h-3.5 w-3.5 text-muted-foreground" />
						)}
						{value ? "true" : "false"}
					</span>
					<CopyButton text={display} />
				</div>
			);

		case "vector": {
			const arr = ensureNumericArray(value);
			return (
				<div className="group min-w-0 space-y-1.5">
					<div className="flex min-w-0 items-center justify-between gap-2">
						<Badge variant="outline" className="text-[10px] px-1.5 py-0">
							{t("dimsdVector", "{{dims}}d vector", { dims })}
						</Badge>
						<CopyButton text={display} />
					</div>
					<Sparkline data={arr.slice(0, 128)} />
				</div>
			);
		}

		case "number":
			return (
				<div className="group flex min-w-0 items-start justify-between gap-2">
					<span className="min-w-0 text-sm font-mono [overflow-wrap:anywhere]">
						{typeof value === "number" ? value.toLocaleString() : String(value)}
					</span>
					<CopyButton text={display} />
				</div>
			);

		case "date":
			return (
				<div className="group flex min-w-0 items-center justify-between gap-2">
					<RelativeTime value={value} className="min-w-0 text-sm" />
					<CopyButton text={String(value)} />
				</div>
			);

		case "user":
			return (
				<div className="group flex min-w-0 items-center justify-between gap-2">
					<UserInlineTag
						userId={String(value).trim()}
						className="min-w-0 text-sm"
					/>
					<CopyButton text={String(value)} />
				</div>
			);

		case "array":
		case "object": {
			const json = JSON.stringify(value, null, 2);
			const isLong = json.length > 200;
			return (
				<div className="group relative min-w-0">
					<pre
						className={`max-w-full whitespace-pre-wrap rounded bg-muted/30 p-2 pr-7 text-xs font-mono [overflow-wrap:anywhere] ${isLong ? "max-h-48 overflow-y-auto" : ""}`}
					>
						{json}
					</pre>
					<div className="absolute top-1 right-1">
						<CopyButton text={json} />
					</div>
				</div>
			);
		}

		default: {
			const isLong = display.length > 120;
			return (
				<div className="group flex min-w-0 items-start justify-between gap-2">
					<p
						className={`min-w-0 flex-1 whitespace-pre-wrap text-sm leading-relaxed [overflow-wrap:anywhere] ${compact && isLong ? "line-clamp-3" : ""}`}
					>
						{display}
					</p>
					<CopyButton text={display} />
				</div>
			);
		}
	}
}

function FieldFilter({
	allFields,
	hiddenFields,
	onToggle,
}: {
	allFields: string[];
	hiddenFields: Set<string>;
	onToggle: (field: string) => void;
}) {
	const { t } = useTranslation("common");
	return (
		<Popover>
			<PopoverTrigger asChild>
				<Button
					variant="ghost"
					size="icon"
					className="relative h-8 w-8"
					aria-label={t("filterVisibleFields", "Filter visible fields")}
					title={t("filterVisibleFields", "Filter visible fields")}
				>
					<Filter className="h-4 w-4" />
					{hiddenFields.size > 0 && (
						<span className="absolute -top-1 -right-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-[9px] text-primary-foreground">
							{hiddenFields.size}
						</span>
					)}
				</Button>
			</PopoverTrigger>
			<PopoverContent className="w-56 p-2" align="end">
				<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-2 px-1">
					{t("visibleFields", "Visible fields")}
				</p>
				<div className="space-y-1 max-h-60 overflow-y-auto">
					{allFields.map((field) => (
						<button
							key={field}
							type="button"
							className="flex w-full min-w-0 items-center gap-2 rounded px-2 py-1 text-left text-sm transition-colors hover:bg-accent"
							aria-pressed={!hiddenFields.has(field)}
							onClick={() => onToggle(field)}
						>
							{hiddenFields.has(field) ? (
								<EyeOff className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
							) : (
								<Eye className="h-3.5 w-3.5 text-foreground shrink-0" />
							)}
							<span
								className={`min-w-0 [overflow-wrap:anywhere] ${hiddenFields.has(field) ? "text-muted-foreground line-through" : ""}`}
							>
								{field}
							</span>
						</button>
					))}
				</div>
			</PopoverContent>
		</Popover>
	);
}

export interface PropertyRowEdit {
	editability: PropertyEditability;
	editing: boolean;
	disabled: boolean;
	/** The relationships a locked link column carries, named in the lock hint. */
	relationshipLabel?: string;
	onStart(): void;
	onDone(): void;
	onSave(value: unknown): Promise<void>;
}

function PropertyRowAffordance({
	propKey,
	edit,
	editButtonRef,
}: {
	propKey: string;
	edit: PropertyRowEdit;
	editButtonRef: React.Ref<HTMLButtonElement>;
}) {
	if (edit.editing) return null;
	const { editability } = edit;
	if (isEditableProperty(editability)) {
		return (
			<PropertyEditButton
				ref={editButtonRef}
				name={propKey}
				onClick={edit.onStart}
				disabled={edit.disabled}
			/>
		);
	}
	return (
		<PropertyLockHint
			reason={editability.locked}
			kind={editability.kind}
			relationshipLabel={edit.relationshipLabel}
		/>
	);
}

/**
 * Puts focus back on the row's pencil once its editor closes, unless the user
 * has already moved focus somewhere else.
 */
function useFocusAfterEdit(editing: boolean) {
	const editButtonRef = useRef<HTMLButtonElement>(null);
	const wasEditing = useRef(editing);
	useEffect(() => {
		const closed = wasEditing.current && !editing;
		wasEditing.current = editing;
		const button = editButtonRef.current;
		if (!closed || !button) return;
		const active = button.ownerDocument.activeElement;
		if (!active || active.contains(button)) button.focus();
	}, [editing]);
	return editButtonRef;
}

function PropertyRow({
	propKey,
	value,
	metadata,
	typeName,
	edit,
}: {
	propKey: string;
	value: unknown;
	metadata?: Record<string, string>;
	typeName?: string;
	/** Omitted, the row is read-only and renders exactly as it always has. */
	edit?: PropertyRowEdit;
}) {
	const appId = usePropertyStorageAppId();
	const editButtonRef = useFocusAfterEdit(edit?.editing ?? false);
	const kindChip = (
		<span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[9px] text-muted-foreground">
			{inferValueKind(value, propKey, { metadata, typeName, appId }).kind}
		</span>
	);
	return (
		<div
			className={`${edit ? "group " : ""}min-w-0 rounded-lg border border-border/60 bg-muted/20 px-3 py-2.5`}
		>
			<div className="mb-1.5 flex min-w-0 items-start justify-between gap-2">
				<p className="min-w-0 text-[11px] font-medium text-muted-foreground [overflow-wrap:anywhere]">
					{propKey}
				</p>
				{edit ? (
					<div className="-my-1 flex shrink-0 items-center gap-1">
						{kindChip}
						<PropertyRowAffordance
							propKey={propKey}
							edit={edit}
							editButtonRef={editButtonRef}
						/>
					</div>
				) : (
					kindChip
				)}
			</div>
			{edit?.editing ? (
				<InlinePropertyEditor
					name={propKey}
					value={value}
					editability={edit.editability}
					onSave={edit.onSave}
					onDone={edit.onDone}
				/>
			) : (
				<PropertyValue
					value={value}
					propKey={propKey}
					metadata={metadata}
					typeName={typeName}
				/>
			)}
		</div>
	);
}

/** Which relationships a link column on `table` stores, for the lock hint. */
function relationshipLabelsForColumn(
	overlay: GraphOverlay,
	table: string,
	column: string,
): string | undefined {
	const labels = new Set(
		overlay.edges
			.filter(
				(edge) =>
					edge.table === table &&
					(edge.src_column === column || edge.dst_column === column),
			)
			.map((edge) => edge.label),
	);
	return labels.size > 0 ? [...labels].join(", ") : undefined;
}

interface ObjectEditContext {
	identity: EditableIdentity;
	locked: ReadonlyMap<string, ColumnLock> | null;
}

function resolveObjectEditContext(
	overlay: GraphOverlay | undefined,
	node: SubgraphNode,
): ObjectEditContext {
	if (!overlay) {
		return { identity: { ok: false, reason: "unknownType" }, locked: null };
	}
	const identity = resolveObjectIdentity(overlay, node.label, node.props);
	return {
		identity,
		locked: identity.ok ? lockedObjectColumns(overlay, identity.mapping) : null,
	};
}

export type UpdateElementProperties = (
	updates: Record<string, unknown>,
	baseline: Record<string, unknown>,
) => Promise<void>;

interface PropertyRowEditsOptions {
	/** Switching to another element drops the open editor. */
	elementId: string | undefined;
	props: Record<string, unknown> | undefined;
	/** Undefined while column types load, which keeps every row read-only. */
	fields: ReadonlyMap<string, ObjectEditField> | undefined;
	/** Null when the element can't be edited at all. */
	locked: ReadonlyMap<string, ColumnLock> | null;
	onUpdate: UpdateElementProperties | undefined;
	relationshipLabel?: (column: string) => string | undefined;
}

/** One property edits at a time; the others wait until it is saved or cancelled. */
export function usePropertyRowEdits({
	elementId,
	props,
	fields,
	locked,
	onUpdate,
	relationshipLabel,
}: PropertyRowEditsOptions) {
	const [editing, setEditing] = useState<{
		elementId: string;
		key: string;
	} | null>(null);
	const stopEditing = useCallback(() => setEditing(null), []);
	const editingKey =
		elementId !== undefined && editing?.elementId === elementId
			? editing.key
			: null;

	const rowEdit = (
		key: string,
		value: unknown,
	): PropertyRowEdit | undefined => {
		if (!onUpdate || !fields || !locked || elementId === undefined) {
			return undefined;
		}
		const editability = propertyEditability(
			key,
			value,
			fields.get(key),
			locked,
		);
		return {
			editability,
			editing: editingKey === key,
			disabled: editingKey !== null && editingKey !== key,
			relationshipLabel:
				!isEditableProperty(editability) &&
				editability.locked === "relationship"
					? relationshipLabel?.(key)
					: undefined,
			onStart: () => setEditing({ elementId, key }),
			// A save that settles late must not close an editor opened since.
			onDone: () =>
				setEditing((current) =>
					current?.elementId === elementId && current.key === key
						? null
						: current,
				),
			onSave: (next) => onUpdate({ [key]: next }, props ?? {}),
		};
	};

	/** Hiding the property being edited closes its editor, so no row stays blocked. */
	const guardToggle =
		(onToggle: (field: string) => void) => (field: string) => {
			if (field === editingKey) stopEditing();
			onToggle(field);
		};

	return { editingKey, rowEdit, guardToggle };
}

export function GraphNodeInspector({
	node,
	overlay,
	connections,
	onClose,
	onExpand,
	onGuidedExpand,
	focused = false,
	onFocus,
	hasChildren,
	childrenExpanded,
	onExpandChildren,
	onCollapseChildren,
	onConnectionClick,
	onFindPath,
	onRunAction,
	editFields,
	onUpdateProperties,
}: GraphNodeInspectorProps) {
	const { t } = useTranslation("common");
	const [hiddenFields, setHiddenFields] = useState<Set<string>>(new Set());
	const [showAllProps, setShowAllProps] = useState(false);

	const editContext = useMemo(
		() =>
			onUpdateProperties && node
				? resolveObjectEditContext(overlay, node)
				: null,
		[onUpdateProperties, overlay, node],
	);
	const { editingKey, rowEdit, guardToggle } = usePropertyRowEdits({
		elementId: node?.id,
		props: node?.props,
		fields: editFields,
		locked: editContext?.locked ?? null,
		onUpdate: onUpdateProperties,
		relationshipLabel: (column) =>
			overlay && editContext?.identity.ok
				? relationshipLabelsForColumn(
						overlay,
						editContext.identity.mapping.table,
						column,
					)
				: undefined,
	});

	const mapping = useMemo(
		() => overlay?.nodes.find((candidate) => candidate.label === node?.label),
		[overlay, node?.label],
	);
	const typeNames = useMemo(
		() => declaredTypes(mapping?.property_columns),
		[mapping],
	);
	const objectView = useMemo(
		() =>
			mapping
				? overlay?.object_views?.find((view) =>
						objectTypeMatches(mapping, view.object_type),
					)
				: undefined,
		[overlay, mapping],
	);
	const actions = useMemo(
		() =>
			mapping
				? (overlay?.actions?.filter(
						(action) =>
							action.enabled && objectTypeMatches(mapping, action.object_type),
					) ?? [])
				: [],
		[overlay, mapping],
	);

	const handleToggleField = useCallback((field: string) => {
		setHiddenFields((prev) => {
			const next = new Set(prev);
			if (next.has(field)) next.delete(field);
			else next.add(field);
			return next;
		});
	}, []);

	if (!node) return null;

	const Icon = getGraphIcon(node.style?.icon ?? "database");
	const propEntries = node.props
		? Object.entries(node.props).filter(
				([k, v]) => (v !== null && v !== undefined) || k === editingKey,
			)
		: [];
	const allFields = propEntries.map(([k]) => k);
	const visibleEntries = propEntries.filter(([k]) => !hiddenFields.has(k));

	const titleValue = objectView?.title_property
		? node.props?.[objectView.title_property]
		: undefined;
	const headerTitle =
		titleValue !== undefined && titleValue !== null && titleValue !== ""
			? String(titleValue)
			: (node.caption ?? node.id);
	const titleAccountId = nodeCaptionAccountId(node, overlay);

	const prominent = objectView?.prominent_properties ?? [];
	const prominentSet = new Set(prominent);
	const prominentEntries =
		prominent.length > 0
			? prominent
					.map((key) => visibleEntries.find(([entryKey]) => entryKey === key))
					.filter((entry): entry is [string, unknown] => entry !== undefined)
			: [];
	const otherEntries =
		prominent.length > 0
			? visibleEntries.filter(([key]) => !prominentSet.has(key))
			: visibleEntries;
	const collapsedOthers = prominent.length > 0 && !showAllProps;

	return (
		<div className="flex h-full min-h-0 w-80 min-w-0 max-w-full shrink-0 flex-col overflow-hidden border-l bg-background animate-in slide-in-from-right-5 duration-200">
			<div className="flex shrink-0 items-start justify-between gap-3 border-b bg-muted/20 p-4">
				<div className="flex min-w-0 flex-1 items-start gap-3">
					<div
						className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl shadow-sm"
						style={{ backgroundColor: node.style?.color ?? "#64748b" }}
					>
						<Icon className="h-4 w-4 text-white" />
					</div>
					<div className="min-w-0">
						<h3 className="text-sm font-semibold leading-snug [overflow-wrap:anywhere]">
							{titleAccountId ? (
								<UserInlineTag userId={titleAccountId} className="text-sm" />
							) : (
								headerTitle
							)}
						</h3>
						<p className="mt-1 text-xs text-muted-foreground [overflow-wrap:anywhere]">
							{node.label}
						</p>
						{editContext && !editContext.identity.ok && (
							<p className="mt-1 text-[11px] text-muted-foreground/80">
								{t(
									"objectNotEditableHere",
									"This object can't be edited here.",
								)}
							</p>
						)}
					</div>
				</div>
				<div className="flex items-center gap-1 shrink-0">
					{allFields.length > 0 && (
						<FieldFilter
							allFields={allFields}
							hiddenFields={hiddenFields}
							onToggle={guardToggle(handleToggleField)}
						/>
					)}
					<Button
						variant="ghost"
						size="icon"
						className="h-8 w-8"
						aria-label={t("close", "Close")}
						onClick={onClose}
					>
						<X className="h-4 w-4" />
					</Button>
				</div>
			</div>
			<ScrollArea
				className="min-h-0 min-w-0 flex-1"
				viewportClassName="[&>div]:!block [&>div]:w-full [&>div]:min-w-0"
			>
				<div className="w-full min-w-0 space-y-5 p-4">
					{/* Explore actions */}
					{(onExpand ||
						onGuidedExpand ||
						onFindPath ||
						onFocus ||
						(hasChildren && (onExpandChildren || onCollapseChildren))) && (
						<div className="grid min-w-0 grid-cols-2 gap-2">
							{onFocus && (
								<Button
									variant={focused ? "default" : "outline"}
									size="sm"
									className="h-auto min-h-8 min-w-0 justify-start gap-1.5 whitespace-normal px-2.5 py-1.5 text-left text-xs [overflow-wrap:anywhere]"
									onClick={() => onFocus(focused ? null : 1)}
									title={t(
										"showOnlyThisObjectAndItsNeighbors",
										"Show only this object and its neighbors",
									)}
								>
									<Crosshair className="h-3.5 w-3.5" />
									{focused ? t("exitFocus", "Exit focus") : t("focus", "Focus")}
								</Button>
							)}
							{onFocus && focused && (
								<Button
									variant="outline"
									size="sm"
									className="h-auto min-h-8 min-w-0 justify-start gap-1.5 whitespace-normal px-2.5 py-1.5 text-left text-xs [overflow-wrap:anywhere]"
									onClick={() => onFocus(2)}
									title={t(
										"widenTheFocusToTwoHops",
										"Widen the focus to two hops",
									)}
								>
									<Crosshair className="h-3.5 w-3.5" />
									{t("2Hops", "2 hops")}
								</Button>
							)}
							{onExpand && (
								<Button
									variant="outline"
									size="sm"
									className="h-auto min-h-8 min-w-0 justify-start gap-1.5 whitespace-normal px-2.5 py-1.5 text-left text-xs [overflow-wrap:anywhere]"
									onClick={() => onExpand(1)}
									title={t(
										"expandNeighborsShiftclick",
										"Expand neighbors (Shift+Click)",
									)}
								>
									<Expand className="h-3.5 w-3.5" />
									{t("expand", "Expand")}
								</Button>
							)}
							{onExpand && (
								<Button
									variant="outline"
									size="sm"
									className="h-auto min-h-8 min-w-0 justify-start gap-1.5 whitespace-normal px-2.5 py-1.5 text-left text-xs [overflow-wrap:anywhere]"
									onClick={() => onExpand(2)}
									title={t(
										"expandNeighborsUpTo2HopsAway",
										"Expand neighbors up to 2 hops away",
									)}
								>
									<Expand className="h-3.5 w-3.5" />
									{t("2Hops", "2 hops")}
								</Button>
							)}
							{onGuidedExpand && (
								<Button
									variant="outline"
									size="sm"
									className="h-auto min-h-8 min-w-0 justify-start gap-1.5 whitespace-normal px-2.5 py-1.5 text-left text-xs [overflow-wrap:anywhere]"
									onClick={onGuidedExpand}
									title={t(
										"chooseRelationshipsAndALimitBeforeExpanding",
										"Choose relationships and a limit before expanding",
									)}
								>
									<SlidersHorizontal className="h-3.5 w-3.5" />
									{t("expandWith", "Expand with…")}
								</Button>
							)}
							{hasChildren && onExpandChildren && !childrenExpanded && (
								<Button
									variant="outline"
									size="sm"
									className="h-auto min-h-8 min-w-0 justify-start gap-1.5 whitespace-normal px-2.5 py-1.5 text-left text-xs [overflow-wrap:anywhere]"
									onClick={onExpandChildren}
									title={t(
										"expandContainmentChildren",
										"Expand containment children",
									)}
								>
									<ListTree className="h-3.5 w-3.5" />
									{t("expandChildren", "Expand children")}
								</Button>
							)}
							{hasChildren && onCollapseChildren && childrenExpanded && (
								<Button
									variant="outline"
									size="sm"
									className="h-auto min-h-8 min-w-0 justify-start gap-1.5 whitespace-normal px-2.5 py-1.5 text-left text-xs [overflow-wrap:anywhere]"
									onClick={onCollapseChildren}
									title={t(
										"collapseContainmentChildren",
										"Collapse containment children",
									)}
								>
									<ChevronsDownUp className="h-3.5 w-3.5" />
									{t("collapse", "Collapse")}
								</Button>
							)}
							{onFindPath && (
								<Button
									variant="outline"
									size="sm"
									className="h-auto min-h-8 min-w-0 justify-start gap-1.5 whitespace-normal px-2.5 py-1.5 text-left text-xs [overflow-wrap:anywhere]"
									onClick={() => onFindPath(node)}
									title="Find a path from this object to another"
								>
									<Route className="h-3.5 w-3.5" />
									Find path from here
								</Button>
							)}
						</div>
					)}

					{/* Ontology actions */}
					{onRunAction && actions.length > 0 && (
						<div>
							<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-2">
								{t("actions", "Actions")}
							</p>
							<div className="flex min-w-0 flex-wrap gap-2">
								{actions.map((action) => (
									<Button
										key={action.id}
										variant="outline"
										size="sm"
										className="h-auto min-h-8 min-w-0 max-w-full justify-start gap-1.5 whitespace-normal px-2.5 py-1.5 text-left text-xs [overflow-wrap:anywhere]"
										onClick={() => onRunAction(action, node)}
										title={action.description ?? action.name}
									>
										<Workflow className="h-3.5 w-3.5" />
										{action.name}
									</Button>
								))}
							</div>
						</div>
					)}

					<div>
						<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-1">
							ID
						</p>
						<div className="group flex min-w-0 items-start justify-between gap-2">
							<p className="min-w-0 text-xs font-mono leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">
								{node.id}
							</p>
							<CopyButton text={node.id} />
						</div>
					</div>
					{visibleEntries.length > 0 && (
						<div>
							<div className="flex items-center justify-between mb-2">
								<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
									{t("properties", "Properties")}
								</p>
								{hiddenFields.size > 0 && (
									<span className="text-[10px] text-muted-foreground">
										{t("sizeHidden", "{{size}} hidden", {
											size: hiddenFields.size,
										})}
									</span>
								)}
							</div>
							<div className="space-y-2">
								{prominentEntries.map(([key, value]) => (
									<PropertyRow
										key={`${node.id}:${key}`}
										propKey={key}
										value={value}
										metadata={node.property_metadata?.[key]}
										typeName={typeNames.get(key)}
										edit={rowEdit(key, value)}
									/>
								))}
								{!collapsedOthers &&
									otherEntries.map(([key, value]) => (
										<PropertyRow
											key={`${node.id}:${key}`}
											propKey={key}
											value={value}
											metadata={node.property_metadata?.[key]}
											typeName={typeNames.get(key)}
											edit={rowEdit(key, value)}
										/>
									))}
							</div>
							{collapsedOthers && otherEntries.length > 0 && (
								<button
									type="button"
									className="mt-2 flex items-center gap-1 text-[11px] font-medium text-muted-foreground hover:text-foreground"
									onClick={() => setShowAllProps(true)}
								>
									<ChevronDown className="h-3.5 w-3.5" />
									{t(
										"showAllPropertiesLength",
										"Show all properties ({{length}})",
										{ length: otherEntries.length },
									)}
								</button>
							)}
						</div>
					)}
					{propEntries.length === 0 && (
						<p className="text-xs text-muted-foreground italic">
							{t("noPropertiesAvailable", "No properties available")}
						</p>
					)}
					{propEntries.length > 0 && visibleEntries.length === 0 && (
						<p className="text-xs text-muted-foreground italic">
							{t(
								"allFieldsHiddenUseTheFilterToShowThem",
								"All fields hidden. Use the filter to show them.",
							)}
						</p>
					)}

					{/* Connections section */}
					{connections && connections.length > 0 && (
						<div>
							<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-2">
								{t("connectionsLength", "Connections ({{length}})", {
									length: connections.length,
								})}
							</p>
							<div className="space-y-1">
								{connections.map((conn, i) => (
									<button
										type="button"
										key={`${conn.direction}-${conn.label}-${conn.targetId}-${i}`}
										className="flex w-full min-w-0 items-start gap-2 rounded-lg border border-border/60 bg-muted/20 px-3 py-2 text-left text-xs transition-colors hover:bg-accent disabled:cursor-default disabled:hover:bg-muted/20"
										disabled={!onConnectionClick}
										onClick={() => onConnectionClick?.(conn.targetId)}
									>
										<span
											className={`shrink-0 text-[10px] font-medium ${conn.direction === "outgoing" ? "text-blue-500" : "text-amber-500"}`}
										>
											{conn.direction === "outgoing" ? "→" : "←"}
										</span>
										<span className="min-w-0 flex-1 space-y-1">
											<span className="block text-[10px] font-medium text-muted-foreground [overflow-wrap:anywhere]">
												{conn.label}
											</span>
											{conn.targetAccountId ? (
												<UserInlineTag
													userId={conn.targetAccountId}
													className="min-w-0 text-xs"
												/>
											) : (
												<span className="block leading-relaxed [overflow-wrap:anywhere]">
													{conn.targetCaption}
												</span>
											)}
										</span>
									</button>
								))}
							</div>
						</div>
					)}
				</div>
			</ScrollArea>
		</div>
	);
}

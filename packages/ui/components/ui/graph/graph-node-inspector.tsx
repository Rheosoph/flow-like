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
	Workflow,
	X,
} from "lucide-react";
import type React from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { inferTemporalValue } from "../../../lib/date";
import type {
	GraphOverlay,
	NodeLabelMapping,
	OntologyActionDefinition,
	SubgraphNode,
} from "../../../state/backend-state/graph-state";
import { Badge } from "../badge";
import { Button } from "../button";
import { Checkbox } from "../checkbox";
import { Popover, PopoverContent, PopoverTrigger } from "../popover";
import { RelativeTime } from "../relative-time";
import { ScrollArea } from "../scroll-area";
import { getGraphIcon } from "./icons";

export interface ConnectionInfo {
	label: string;
	direction: "outgoing" | "incoming";
	targetCaption: string;
	targetId: string;
}

export interface GraphNodeInspectorProps {
	node: SubgraphNode | null;
	overlay?: GraphOverlay;
	connections?: ConnectionInfo[];
	onClose: () => void;
	onExpand?: (depth: number) => void;
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
	| "string"
	| "number"
	| "boolean"
	| "date"
	| "vector"
	| "array"
	| "object"
	| "unknown";

export { inferValueKind, PropertyValue, FieldFilter, CopyButton };

/**
 * `propKey` is what makes an epoch integer readable: an ontology property is
 * untyped by the time it reaches here, so a `created_at` holding 1786353300000
 * is indistinguishable from a quantity without its name.
 */
function inferValueKind(
	value: unknown,
	propKey?: string,
): { kind: ValueKind; dims?: number } {
	if (value === null || value === undefined) return { kind: "unknown" };
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
			className="opacity-0 group-hover:opacity-100 transition-opacity p-0.5 rounded hover:bg-accent shrink-0"
			title={t('copyValue', 'Copy value')}
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
}: { value: unknown; propKey: string }) {
	const { t } = useTranslation("common");
	const { kind, dims } = inferValueKind(value, propKey);
	const display =
		typeof value === "object"
			? JSON.stringify(value, null, 2)
			: String(value ?? "—");

	switch (kind) {
		case "boolean":
			return (
				<div className="group flex items-center justify-between">
					<div className="flex items-center gap-2">
						<Checkbox checked={value as boolean} disabled className="h-4 w-4" />
						<span className="text-sm">{value ? "true" : "false"}</span>
					</div>
					<CopyButton text={display} />
				</div>
			);

		case "vector": {
			const arr = ensureNumericArray(value);
			return (
				<div className="group space-y-1.5">
					<div className="flex items-center justify-between">
						<Badge variant="outline" className="text-[10px] px-1.5 py-0">{t('dimsdVector', '{{dims}}d vector', { dims })}</Badge>
						<CopyButton text={display} />
					</div>
					<Sparkline data={arr.slice(0, 128)} />
				</div>
			);
		}

		case "number":
			return (
				<div className="group flex items-center justify-between">
					<span className="text-sm font-mono">
						{typeof value === "number" ? value.toLocaleString() : String(value)}
					</span>
					<CopyButton text={display} />
				</div>
			);

		case "date":
			return (
				<div className="group flex items-center justify-between">
					<RelativeTime value={value} className="text-sm" />
					<CopyButton text={String(value)} />
				</div>
			);

		case "array":
		case "object": {
			const json = JSON.stringify(value, null, 2);
			const isLong = json.length > 200;
			return (
				<div className="group relative">
					<pre
						className={`text-xs font-mono break-all whitespace-pre-wrap bg-muted/30 rounded p-1.5 ${isLong ? "max-h-32 overflow-y-auto" : ""}`}
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
				<div className="group flex items-start justify-between gap-1">
					<p className={`text-sm break-all ${isLong ? "line-clamp-3" : ""}`}>
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
					className="h-7 w-7"
					title={t('filterVisibleFields', 'Filter visible fields')}
				>
					<Filter className="h-4 w-4" />
					{hiddenFields.size > 0 && (
						<span className="absolute -top-0.5 -right-0.5 h-3 w-3 rounded-full bg-primary text-[8px] text-primary-foreground flex items-center justify-center">
							{hiddenFields.size}
						</span>
					)}
				</Button>
			</PopoverTrigger>
			<PopoverContent className="w-56 p-2" align="end">
				<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-2 px-1">
					{t('visibleFields', 'Visible fields')}
				</p>
				<div className="space-y-1 max-h-60 overflow-y-auto">
					{allFields.map((field) => (
						<button
							key={field}
							type="button"
							className="w-full flex items-center gap-2 px-2 py-1 rounded text-sm hover:bg-accent transition-colors text-left"
							onClick={() => onToggle(field)}
						>
							{hiddenFields.has(field) ? (
								<EyeOff className="h-3.5 w-3.5 text-muted-foreground shrink-0" />
							) : (
								<Eye className="h-3.5 w-3.5 text-foreground shrink-0" />
							)}
							<span
								className={
									hiddenFields.has(field)
										? "text-muted-foreground line-through"
										: ""
								}
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

function PropertyRow({ propKey, value }: { propKey: string; value: unknown }) {
	return (
		<div className="rounded-md bg-muted/50 px-3 py-2">
			<div className="flex items-center justify-between mb-0.5">
				<p className="text-[10px] font-medium text-muted-foreground">
					{propKey}
				</p>
				<span className="text-[9px] text-muted-foreground/60">
					{inferValueKind(value, propKey).kind}
				</span>
			</div>
			<PropertyValue value={value} propKey={propKey} />
		</div>
	);
}

export function GraphNodeInspector({
	node,
	overlay,
	connections,
	onClose,
	onExpand,
	onFocus,
	focused = false,
	hasChildren,
	childrenExpanded,
	onExpandChildren,
	onCollapseChildren,
	onConnectionClick,
	onFindPath,
	onRunAction,
}: GraphNodeInspectorProps) {
	const { t } = useTranslation("common");
	const [hiddenFields, setHiddenFields] = useState<Set<string>>(new Set());
	const [showAllProps, setShowAllProps] = useState(false);

	const mapping = useMemo(
		() => overlay?.nodes.find((candidate) => candidate.label === node?.label),
		[overlay, node?.label],
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
				([, v]) => v !== null && v !== undefined,
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
		<div className="w-80 shrink-0 bg-background border-l flex flex-col h-full min-h-0 overflow-hidden animate-in slide-in-from-right-5 duration-200">
			<div className="flex items-center justify-between p-4 border-b shrink-0">
				<div className="flex items-center gap-2 min-w-0">
					<div
						className="w-7 h-7 rounded-full flex items-center justify-center shrink-0 shadow-sm"
						style={{ backgroundColor: node.style?.color ?? "#64748b" }}
					>
						<Icon className="h-3.5 w-3.5 text-white" />
					</div>
					<div className="min-w-0">
						<h3 className="font-semibold text-sm truncate">{headerTitle}</h3>
						<p className="text-xs text-muted-foreground">{node.label}</p>
					</div>
				</div>
				<div className="flex items-center gap-1 shrink-0">
					{allFields.length > 0 && (
						<FieldFilter
							allFields={allFields}
							hiddenFields={hiddenFields}
							onToggle={handleToggleField}
						/>
					)}
					<Button
						variant="ghost"
						size="icon"
						className="h-7 w-7"
						onClick={onClose}
					>
						<X className="h-4 w-4" />
					</Button>
				</div>
			</div>
			<ScrollArea className="flex-1 min-h-0">
				<div className="space-y-4 p-4">
					{/* Explore actions */}
					{(onExpand ||
						onFindPath ||
						onFocus ||
						(hasChildren && (onExpandChildren || onCollapseChildren))) && (
						<div className="flex flex-wrap gap-1.5">
							{onFocus && (
								<Button
									variant={focused ? "default" : "outline"}
									size="sm"
									className="h-7 gap-1.5 text-xs"
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
									className="h-7 gap-1.5 text-xs"
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
									className="h-7 gap-1.5 text-xs"
									onClick={() => onExpand(1)}
									title={t('expandNeighborsShiftclick', 'Expand neighbors (Shift+Click)')}
								>
									<Expand className="h-3.5 w-3.5" />
									{t('expand', 'Expand')}
								</Button>
							)}
							{onExpand && (
								<Button
									variant="outline"
									size="sm"
									className="h-7 gap-1.5 text-xs"
									onClick={() => onExpand(2)}
									title={t('expandNeighborsUpTo2HopsAway', 'Expand neighbors up to 2 hops away')}
								>
									<Expand className="h-3.5 w-3.5" />{t('2Hops', '2 hops')}
								</Button>
							)}
							{hasChildren && onExpandChildren && !childrenExpanded && (
								<Button
									variant="outline"
									size="sm"
									className="h-7 gap-1.5 text-xs"
									onClick={onExpandChildren}
									title={t('expandContainmentChildren', 'Expand containment children')}
								>
									<ListTree className="h-3.5 w-3.5" />
									{t('expandChildren', 'Expand children')}
								</Button>
							)}
							{hasChildren && onCollapseChildren && childrenExpanded && (
								<Button
									variant="outline"
									size="sm"
									className="h-7 gap-1.5 text-xs"
									onClick={onCollapseChildren}
									title={t('collapseContainmentChildren', 'Collapse containment children')}
								>
									<ChevronsDownUp className="h-3.5 w-3.5" />
									{t('collapse', 'Collapse')}
								</Button>
							)}
							{onFindPath && (
								<Button
									variant="outline"
									size="sm"
									className="h-7 gap-1.5 text-xs"
									onClick={() => onFindPath(node)}
									title={`Find a path from this object to another`}
								>
									<Route className="h-3.5 w-3.5" />
									{`Find path from here`}
								</Button>
							)}
						</div>
					)}

					{/* Ontology actions */}
					{onRunAction && actions.length > 0 && (
						<div>
							<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-2">
								{t('actions', 'Actions')}
							</p>
							<div className="flex flex-wrap gap-1.5">
								{actions.map((action) => (
									<Button
										key={action.id}
										variant="outline"
										size="sm"
										className="h-7 gap-1.5 text-xs"
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
						<p className="text-xs font-mono break-all text-muted-foreground">
							{node.id}
						</p>
					</div>
					{visibleEntries.length > 0 && (
						<div>
							<div className="flex items-center justify-between mb-2">
								<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
									{t('properties', 'Properties')}
								</p>
								{hiddenFields.size > 0 && (
									<span className="text-[10px] text-muted-foreground">{t('sizeHidden', '{{size}} hidden', { size: hiddenFields.size })}</span>
								)}
							</div>
							<div className="space-y-2">
								{prominentEntries.map(([key, value]) => (
									<PropertyRow key={key} propKey={key} value={value} />
								))}
								{!collapsedOthers &&
									otherEntries.map(([key, value]) => (
										<PropertyRow key={key} propKey={key} value={value} />
									))}
							</div>
							{collapsedOthers && otherEntries.length > 0 && (
								<button
									type="button"
									className="mt-2 flex items-center gap-1 text-[11px] font-medium text-muted-foreground hover:text-foreground"
									onClick={() => setShowAllProps(true)}
								>
									<ChevronDown className="h-3.5 w-3.5" />{t('showAllPropertiesLength', 'Show all properties ({{length}})', { length: otherEntries.length })}</button>
							)}
						</div>
					)}
					{propEntries.length === 0 && (
						<p className="text-xs text-muted-foreground italic">
							{t('noPropertiesAvailable', 'No properties available')}
						</p>
					)}
					{propEntries.length > 0 && visibleEntries.length === 0 && (
						<p className="text-xs text-muted-foreground italic">
							{t('allFieldsHiddenUseTheFilterToShowThem', 'All fields hidden — use the filter to show them')}
						</p>
					)}

					{/* Connections section */}
					{connections && connections.length > 0 && (
						<div>
							<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-2">{t('connectionsLength', 'Connections ({{length}})', { length: connections.length })}</p>
							<div className="space-y-1">
								{connections.map((conn, i) => (
									<button
										type="button"
										key={`${conn.direction}-${conn.label}-${conn.targetId}-${i}`}
										className="w-full rounded-md bg-muted/50 px-3 py-1.5 flex items-center gap-2 text-xs hover:bg-accent transition-colors text-left cursor-pointer"
										onClick={() => onConnectionClick?.(conn.targetId)}
									>
										<span
											className={`shrink-0 text-[10px] font-medium ${conn.direction === "outgoing" ? "text-blue-500" : "text-amber-500"}`}
										>
											{conn.direction === "outgoing" ? "→" : "←"}
										</span>
										<span className="font-medium text-muted-foreground shrink-0">
											{conn.label}
										</span>
										<span className="truncate">{conn.targetCaption}</span>
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

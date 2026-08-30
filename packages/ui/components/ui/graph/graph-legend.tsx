"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronDown, ChevronUp, Eye, EyeOff } from "lucide-react";
import { useCallback, useMemo, useRef, useState } from "react";
import type { LabelStyle } from "../../../state/backend-state/graph-state";
import { Popover, PopoverContent, PopoverTrigger } from "../popover";
import { GRAPH_ICONS, type IconKey, getGraphIcon } from "./icons";

export interface LegendEntry {
	label: string;
	style: LabelStyle;
	count?: number;
	/** Whole-population size, when it is known exactly for this label. */
	total?: number;
	type: "node" | "edge";
}

export interface GraphLegendProps {
	entries: LegendEntry[];
	/**
	 * Controlled hidden-label set. Omit to let the legend own it, which the a2ui
	 * `graph` element relies on. Supplying it makes the caller the single source of
	 * truth, so label visibility set from outside the legend stays in sync with it.
	 */
	hidden?: ReadonlySet<string>;
	onToggleVisibility?: (label: string, visible: boolean) => void;
	onStyleChange?: (
		label: string,
		type: "node" | "edge",
		style: LabelStyle,
	) => void;
}

/**
 * Rows a section shows before it asks. An ontology can map dozens of labels, and
 * a legend that lists every one of them covers the graph it is annotating.
 */
const COLLAPSED_ROWS = 6;
/** Past this the whole panel scrolls rather than growing up the canvas. */
const BODY_MAX_HEIGHT = "min(46vh, 340px)";

function InlineColorPicker({
	color,
	onChange,
}: { color: string; onChange: (color: string) => void }) {
	const { t } = useTranslation("common");
	const inputRef = useRef<HTMLInputElement>(null);
	return (
		<button
			type="button"
			className="relative w-4 h-4 rounded shrink-0 border border-border/50 cursor-pointer"
			style={{ backgroundColor: color }}
			onClick={(e) => {
				e.stopPropagation();
				inputRef.current?.click();
			}}
			title={t("changeColor", "Change color")}
		>
			<input
				ref={inputRef}
				type="color"
				value={color}
				onChange={(e) => onChange(e.target.value)}
				onClick={(e) => e.stopPropagation()}
				className="absolute inset-0 w-full h-full opacity-0 cursor-pointer"
			/>
		</button>
	);
}

function InlineIconPicker({
	icon,
	color,
	onChange,
}: { icon: string; color: string; onChange: (icon: string) => void }) {
	const { t } = useTranslation("common");
	const [open, setOpen] = useState(false);
	const Icon = getGraphIcon(icon);
	const iconKeys = Object.keys(GRAPH_ICONS) as IconKey[];

	// A portalled popover, because the list it sits in scrolls: an absolutely
	// positioned panel would be clipped by the scroll container.
	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<button
					type="button"
					className="w-4 h-4 rounded-full shrink-0 flex items-center justify-center ring-2 ring-transparent hover:ring-accent transition-all"
					style={{ backgroundColor: color }}
					title={t("changeIcon", "Change icon")}
				>
					<Icon className="h-2.5 w-2.5 text-white" />
				</button>
			</PopoverTrigger>
			<PopoverContent
				align="start"
				side="top"
				className="w-46 p-2 grid grid-cols-6 gap-1"
			>
				{iconKeys.map((key) => {
					const ItemIcon = GRAPH_ICONS[key];
					const isActive = key === icon;
					return (
						<button
							type="button"
							key={key}
							className={`w-6 h-6 rounded flex items-center justify-center transition-colors ${isActive ? "bg-primary text-primary-foreground" : "hover:bg-accent"}`}
							onClick={() => {
								onChange(key);
								setOpen(false);
							}}
							title={key}
						>
							<ItemIcon className="h-3.5 w-3.5" />
						</button>
					);
				})}
			</PopoverContent>
		</Popover>
	);
}

function VisibilityToggle({
	hidden,
	onClick,
	label,
}: { hidden: boolean; onClick: () => void; label: string }) {
	return (
		<button
			type="button"
			onClick={onClick}
			className="shrink-0 text-muted-foreground hover:text-foreground transition-colors"
			title={label}
			aria-label={label}
		>
			{hidden ? (
				<EyeOff className="h-3 w-3" />
			) : (
				<Eye className="h-3 w-3 opacity-60" />
			)}
		</button>
	);
}

function LegendRow({
	entry,
	hidden,
	onToggle,
	onStyleChange,
}: {
	entry: LegendEntry;
	hidden: boolean;
	onToggle: () => void;
	onStyleChange?: (style: LabelStyle) => void;
}) {
	const { t } = useTranslation("common");
	const Icon = getGraphIcon(entry.style.icon);
	const isEdge = entry.type === "edge";

	return (
		<div className="flex items-center gap-1.5 text-xs">
			{onStyleChange ? (
				<div className="flex items-center gap-1 shrink-0">
					{!isEdge && (
						<InlineIconPicker
							icon={entry.style.icon}
							color={entry.style.color}
							onChange={(icon) => onStyleChange({ ...entry.style, icon })}
						/>
					)}
					<InlineColorPicker
						color={entry.style.color}
						onChange={(color) => onStyleChange({ ...entry.style, color })}
					/>
				</div>
			) : isEdge ? (
				<div
					className="w-3 h-0.5 shrink-0"
					style={{ backgroundColor: entry.style.color }}
				/>
			) : (
				<div
					className="w-3 h-3 rounded-full shrink-0 flex items-center justify-center"
					style={{ backgroundColor: entry.style.color }}
				>
					<Icon className="h-2 w-2 text-white" />
				</div>
			)}
			<span
				className={`min-w-0 flex-1 truncate ${hidden ? "text-muted-foreground line-through" : ""}`}
				title={entry.label}
			>
				{entry.label}
			</span>
			{entry.count !== undefined && (
				<span className="shrink-0 text-muted-foreground tabular-nums">
					{entry.count.toLocaleString()}
					{entry.total !== undefined && (
						<span className="opacity-60">
							{" / "}
							{entry.total.toLocaleString()}
						</span>
					)}
				</span>
			)}
			<VisibilityToggle
				hidden={hidden}
				onClick={onToggle}
				label={hidden ? t("show", "Show") : t("hide", "Hide")}
			/>
		</div>
	);
}

function LegendSection({
	title,
	entries,
	hiddenLabels,
	onToggle,
	onToggleAll,
	onStyleChange,
}: {
	title: string;
	entries: LegendEntry[];
	hiddenLabels: ReadonlySet<string>;
	onToggle: (label: string) => void;
	onToggleAll: (entries: LegendEntry[], visible: boolean) => void;
	onStyleChange?: (
		label: string,
		type: "node" | "edge",
		style: LabelStyle,
	) => void;
}) {
	const { t } = useTranslation("common");
	const [expanded, setExpanded] = useState(false);

	if (entries.length === 0) return null;

	const overflow = entries.length - COLLAPSED_ROWS;
	const shown = expanded ? entries : entries.slice(0, COLLAPSED_ROWS);
	const anyVisible = entries.some((entry) => !hiddenLabels.has(entry.label));

	return (
		<div className="space-y-1">
			<div className="flex items-center gap-1.5">
				<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
					{title}
				</p>
				<span className="text-[10px] text-muted-foreground/60 tabular-nums">
					{entries.length}
				</span>
				<div className="ml-auto">
					<VisibilityToggle
						hidden={!anyVisible}
						onClick={() => onToggleAll(entries, !anyVisible)}
						label={
							anyVisible ? t("hideAll", "Hide all") : t("showAll", "Show all")
						}
					/>
				</div>
			</div>
			{shown.map((entry) => (
				<LegendRow
					key={`${entry.type}-${entry.label}`}
					entry={entry}
					hidden={hiddenLabels.has(entry.label)}
					onToggle={() => onToggle(entry.label)}
					onStyleChange={
						onStyleChange
							? (style) => onStyleChange(entry.label, entry.type, style)
							: undefined
					}
				/>
			))}
			{overflow > 0 && (
				<button
					type="button"
					className="text-[10px] text-muted-foreground hover:text-foreground transition-colors"
					onClick={() => setExpanded(!expanded)}
				>
					{expanded
						? t("showLess", "Show less")
						: t("countMore", "{{count}} more", { count: overflow })}
				</button>
			)}
		</div>
	);
}

/** Most-present labels first, so a truncated section is the useful part of it. */
function byPresence(a: LegendEntry, b: LegendEntry): number {
	return (b.count ?? -1) - (a.count ?? -1) || a.label.localeCompare(b.label);
}

export function GraphLegend({
	entries,
	hidden: hiddenProp,
	onToggleVisibility,
	onStyleChange,
}: GraphLegendProps) {
	const { t } = useTranslation("common");
	const [internalHidden, setInternalHidden] = useState<Set<string>>(new Set());
	const [collapsed, setCollapsed] = useState(false);

	const isControlled = hiddenProp !== undefined;
	const hidden = hiddenProp ?? internalHidden;

	// Notification is driven off the EFFECTIVE set, never off the internal updater's
	// `prev`: under control that value is stale and would invert the reported state.
	const toggle = useCallback(
		(label: string) => {
			const visible = hidden.has(label);
			onToggleVisibility?.(label, visible);
			if (isControlled) return;
			setInternalHidden((prev) => {
				const next = new Set(prev);
				if (visible) next.delete(label);
				else next.add(label);
				return next;
			});
		},
		[hidden, isControlled, onToggleVisibility],
	);

	const toggleAll = useCallback(
		(section: LegendEntry[], visible: boolean) => {
			const affected = section.filter(
				(entry) => hidden.has(entry.label) === visible,
			);
			for (const entry of affected) onToggleVisibility?.(entry.label, visible);
			if (isControlled) return;
			setInternalHidden((prev) => {
				const next = new Set(prev);
				for (const entry of affected) {
					if (visible) next.delete(entry.label);
					else next.add(entry.label);
				}
				return next;
			});
		},
		[hidden, isControlled, onToggleVisibility],
	);

	const { nodeEntries, edgeEntries } = useMemo(
		() => ({
			nodeEntries: entries.filter((e) => e.type === "node").sort(byPresence),
			edgeEntries: entries.filter((e) => e.type === "edge").sort(byPresence),
		}),
		[entries],
	);

	if (entries.length === 0) return null;

	return (
		<div className="w-58 rounded-lg border bg-background/80 shadow-sm backdrop-blur-sm">
			<button
				type="button"
				className="flex w-full items-center gap-2 px-3 py-2 text-left"
				onClick={() => setCollapsed(!collapsed)}
				aria-expanded={!collapsed}
			>
				<span className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
					{t("legend", "Legend")}
				</span>
				<span className="ml-auto text-[10px] text-muted-foreground/60 tabular-nums">
					{t("countLabels", {
						defaultValue_one: "{{count}} label",
						defaultValue_other: "{{count}} labels",
						count: entries.length,
					})}
				</span>
				{collapsed ? (
					<ChevronUp className="h-3 w-3 shrink-0 text-muted-foreground" />
				) : (
					<ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
				)}
			</button>
			{!collapsed && (
				<div
					className="space-y-3 overflow-y-auto px-3 pb-3"
					style={{ maxHeight: BODY_MAX_HEIGHT }}
				>
					<LegendSection
						title={t("nodes2", "Nodes")}
						entries={nodeEntries}
						hiddenLabels={hidden}
						onToggle={toggle}
						onToggleAll={toggleAll}
						onStyleChange={onStyleChange}
					/>
					<LegendSection
						title={t("edges", "Edges")}
						entries={edgeEntries}
						hiddenLabels={hidden}
						onToggle={toggle}
						onToggleAll={toggleAll}
						onStyleChange={onStyleChange}
					/>
				</div>
			)}
		</div>
	);
}

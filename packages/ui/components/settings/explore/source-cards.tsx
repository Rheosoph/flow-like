"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangle,
	Boxes,
	Database,
	Globe,
	MoreVertical,
	Plus,
	Search,
	Trash2,
	User,
} from "lucide-react";
import type React from "react";
import { useCallback, useMemo, useState } from "react";
import { cn } from "../../../lib/utils";
import type {
	IColumnFamily,
	IColumnSummary,
	ITableSummary,
} from "../../../state/backend-state/db-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Card } from "../../ui/card";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../ui/dropdown-menu";

export interface SourceEntry {
	name: string;
	userScoped?: boolean;
	summary?: ITableSummary;
}

/** Facets are derived from the summary, so they only narrow once it has loaded. */
export type SourceFacet =
	| "all"
	| "attention"
	| "unmapped"
	| "vector"
	| "unused";

/**
 * One colour per column family. These are data encodings, deliberately outside
 * the brand accent so an indexed vector column never reads as a primary action.
 */
const FAMILY_CLASS: Record<IColumnFamily, string> = {
	text: "bg-slate-400 dark:bg-slate-400",
	number: "bg-blue-500 dark:bg-blue-400",
	time: "bg-violet-500 dark:bg-violet-400",
	bool: "bg-emerald-500 dark:bg-emerald-400",
	vector: "bg-cyan-500 dark:bg-cyan-400",
	struct: "bg-amber-500 dark:bg-amber-400",
	binary: "bg-pink-500 dark:bg-pink-400",
	other: "bg-muted-foreground",
};

/** LanceDB compacts below this many small fragments without much benefit. */
const FRAGMENT_ALERT_THRESHOLD = 10;
/** Below this row count a missing index costs nothing worth warning about. */
const UNINDEXED_ROW_THRESHOLD = 1000;

type AlertKind = "warn" | "info";

interface SourceAlert {
	kind: AlertKind;
	message: string;
	action: string;
}

export function summaryAlert(
	summary: ITableSummary | undefined,
	t: (
		key: string,
		fallback: string,
		values?: Record<string, unknown>,
	) => string,
): SourceAlert | null {
	if (!summary) return null;
	if (summary.error) {
		return {
			kind: "warn",
			message: summary.error,
			action: t("retry", "Retry"),
		};
	}
	const small = summary.storage?.num_small_fragments ?? 0;
	if (small >= FRAGMENT_ALERT_THRESHOLD) {
		return {
			kind: "warn",
			message: t(
				"countSmallFragmentsSlowEveryRead",
				"{{count}} small fragments slow every read",
				{ count: small },
			),
			action: t("optimize", "Optimize"),
		};
	}
	if (!summary.consumers.object_type) {
		return {
			kind: "info",
			message: t("notInTheSemanticLayer", "Not in the semantic layer"),
			action: t("mapToObjectType", "Map to object type"),
		};
	}
	if (
		summary.indexes.length === 0 &&
		(summary.rows ?? 0) > UNINDEXED_ROW_THRESHOLD
	) {
		return {
			kind: "warn",
			message: t(
				"noIndexFiltersScanEveryRow",
				"No index — filters scan every row",
			),
			action: t("buildIndex", "Build index"),
		};
	}
	return null;
}

export function matchesFacet(entry: SourceEntry, facet: SourceFacet): boolean {
	const summary = entry.summary;
	switch (facet) {
		case "attention":
			return Boolean(summary && summaryAlertKind(summary));
		case "unmapped":
			return Boolean(summary && !summary.consumers.object_type);
		case "vector":
			return Boolean(
				summary?.columns.some((column) => column.family === "vector"),
			);
		case "unused":
			return Boolean(summary && consumerCount(summary) === 0);
		default:
			return true;
	}
}

/** Facet counting must not depend on translated copy, so it re-derives the kind. */
function summaryAlertKind(summary: ITableSummary): AlertKind | null {
	if (summary.error) return "warn";
	if ((summary.storage?.num_small_fragments ?? 0) >= FRAGMENT_ALERT_THRESHOLD)
		return "warn";
	if (!summary.consumers.object_type) return "info";
	if (
		summary.indexes.length === 0 &&
		(summary.rows ?? 0) > UNINDEXED_ROW_THRESHOLD
	)
		return "warn";
	return null;
}

function consumerCount(summary: ITableSummary): number {
	const { queries, actions, views } = summary.consumers;
	return queries + actions + views;
}

export function countAttention(entries: SourceEntry[]): number {
	return entries.filter(
		(entry) => entry.summary && summaryAlertKind(entry.summary),
	).length;
}

function formatCompact(value: number): string {
	if (value < 1000) return String(value);
	if (value < 1_000_000)
		return `${(value / 1000).toFixed(value < 10_000 ? 1 : 0)}K`;
	if (value < 1_000_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
	return `${(value / 1_000_000_000).toFixed(1)}B`;
}

function formatBytes(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	const units = ["KB", "MB", "GB", "TB"];
	let value = bytes;
	let unit = -1;
	while (value >= 1024 && unit < units.length - 1) {
		value /= 1024;
		unit += 1;
	}
	return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
}

function columnLabel(column: IColumnSummary): string {
	return column.family === "vector" && column.vector_size
		? `vector[${column.vector_size}]`
		: column.family;
}

const Shimmer: React.FC<{ className?: string }> = ({ className }) => (
	<span
		className={cn(
			"inline-block animate-pulse rounded-sm bg-muted align-middle motion-reduce:animate-none",
			className,
		)}
	/>
);

/**
 * One tick per column, coloured by type family. It reads as a fingerprint: a
 * wide fact table, an embedding table and a five-column lookup are
 * distinguishable before a single word is read.
 */
const SchemaStrip: React.FC<{
	columns: IColumnSummary[];
	onHover: (column: IColumnSummary | null) => void;
}> = ({ columns, onHover }) => (
	// Ticks are decorative: the caption below is the accessible channel, and
	// the native title keeps a pointer user from having to read it.
	<div
		className="flex h-5 items-stretch gap-0.5"
		onPointerLeave={() => onHover(null)}
	>
		{columns.map((column) => (
			<span
				key={column.name}
				aria-hidden="true"
				title={`${column.name} · ${columnLabel(column)}`}
				onPointerEnter={() => onHover(column)}
				className={cn(
					"min-w-0.75 flex-1 rounded-[2px] opacity-60 transition-opacity hover:opacity-100",
					FAMILY_CLASS[column.family],
				)}
			/>
		))}
	</div>
);

const Metric: React.FC<{
	label: string;
	children: React.ReactNode;
	className?: string;
}> = ({ label, children, className }) => (
	<div className={cn("flex flex-col gap-0.5 px-4 py-2", className)}>
		<span className="text-[9.5px] font-semibold uppercase tracking-wider text-muted-foreground/70">
			{label}
		</span>
		<span className="text-[15px] font-semibold tabular-nums tracking-tight">
			{children}
		</span>
	</div>
);

interface SourceCardProps {
	entry: SourceEntry;
	loading: boolean;
	onSelect: () => void;
	onRequestDelete: () => void;
	onResolveAlert: (entry: SourceEntry) => void;
}

const SourceCard: React.FC<SourceCardProps> = ({
	entry,
	loading,
	onSelect,
	onRequestDelete,
	onResolveAlert,
}) => {
	const { t } = useTranslation("settings");
	const [hovered, setHovered] = useState<IColumnSummary | null>(null);
	const summary = entry.summary;
	const alert = useMemo(() => summaryAlert(summary, t), [summary, t]);
	const objectColor = summary?.consumers.object_color;
	const unmapped = Boolean(summary && !summary.consumers.object_type);

	const chips: React.ReactNode[] = [];
	if (summary) {
		const { ontology, queries, actions, relations, exposed } =
			summary.consumers;
		if (ontology)
			chips.push(
				<Badge key="ontology" variant="outline" className="text-[10.5px]">
					{ontology}
				</Badge>,
			);
		if (queries)
			chips.push(
				<Badge key="queries" variant="outline" className="text-[10.5px]">
					{t("countQueries", {
						defaultValue_one: "{{count}} query",
						defaultValue_other: "{{count}} queries",
						count: queries,
					})}
				</Badge>,
			);
		if (actions)
			chips.push(
				<Badge key="actions" variant="outline" className="text-[10.5px]">
					{t("countActions", {
						defaultValue_one: "{{count}} action",
						defaultValue_other: "{{count}} actions",
						count: actions,
					})}
				</Badge>,
			);
		if (relations)
			chips.push(
				<Badge key="relations" variant="outline" className="text-[10.5px]">
					{t("countLinks", {
						defaultValue_one: "{{count}} link",
						defaultValue_other: "{{count}} links",
						count: relations,
					})}
				</Badge>,
			);
		if (exposed)
			chips.push(
				<Badge key="exposed" variant="outline" className="text-[10.5px]">
					{t("sharedOut", "Shared out")}
				</Badge>,
			);
	}

	return (
		<Card
			className={cn(
				"group relative overflow-hidden border p-0 transition-colors duration-200 hover:border-primary/50",
				unmapped && "border-dashed",
			)}
		>
			{alert && (
				<span
					aria-hidden="true"
					className={cn(
						"absolute inset-y-0 left-0 w-0.75",
						alert.kind === "warn" ? "bg-amber-500" : "bg-sky-500",
					)}
				/>
			)}
			<button
				type="button"
				onClick={onSelect}
				className="w-full cursor-pointer text-left"
				title={t("openTableName", "Open table: {{name}}", { name: entry.name })}
			>
				<div className="flex items-start gap-2.5 px-4 pt-3.5 pr-10">
					<span
						className={cn(
							"grid size-8.5 shrink-0 place-items-center rounded-lg",
							objectColor ? "" : "bg-primary/10 text-primary",
							unmapped &&
								"border border-dashed bg-transparent text-muted-foreground",
						)}
						style={
							objectColor
								? {
										backgroundColor: `color-mix(in oklab, ${objectColor} 15%, transparent)`,
										color: objectColor,
									}
								: undefined
						}
					>
						{unmapped ? (
							<Database className="size-4" />
						) : (
							<Boxes className="size-4" />
						)}
					</span>
					<span className="flex min-w-0 flex-1 flex-col gap-1">
						<span className="truncate font-mono text-[13.5px] font-semibold tracking-tight">
							{entry.name}
						</span>
						<span className="flex flex-wrap items-center gap-1.5">
							{summary ? (
								summary.consumers.object_type ? (
									<Badge
										variant="outline"
										className="text-[10.5px]"
										style={
											objectColor
												? {
														color: objectColor,
														borderColor: `color-mix(in oklab, ${objectColor} 40%, transparent)`,
														backgroundColor: `color-mix(in oklab, ${objectColor} 12%, transparent)`,
													}
												: undefined
										}
									>
										{summary.consumers.object_type}
									</Badge>
								) : (
									<Badge
										variant="outline"
										className="border-dashed border-sky-500/45 bg-sky-500/10 text-[10.5px] text-sky-600 dark:text-sky-400"
									>
										{t("unmapped", "Unmapped")}
									</Badge>
								)
							) : (
								<Shimmer className="h-4 w-16" />
							)}
							{entry.userScoped ? (
								<Badge
									variant="outline"
									className="gap-1 border-amber-500/20 bg-amber-500/10 text-[10.5px] text-amber-500"
								>
									<User className="size-3" />
									{t("userScoped", "User scoped")}
								</Badge>
							) : (
								<Badge
									variant="outline"
									className="gap-1 border-primary/20 bg-primary/10 text-[10.5px] text-primary"
								>
									<Globe className="size-3" />
									{t("shared", "Shared")}
								</Badge>
							)}
						</span>
					</span>
				</div>

				<div className="flex flex-col gap-1.5 px-4 pt-3">
					{summary && summary.columns.length > 0 ? (
						<SchemaStrip columns={summary.columns} onHover={setHovered} />
					) : (
						<Shimmer className="h-5 w-full" />
					)}
					<div className="flex min-h-4 items-center justify-between gap-2 text-[11px] text-muted-foreground">
						{hovered ? (
							<span className="truncate font-mono text-foreground">
								{hovered.name}{" "}
								<span className="text-muted-foreground">
									{columnLabel(hovered)}
									{hovered.nullable ? "" : ` · ${t("required", "required")}`}
								</span>
							</span>
						) : (
							<span>
								{summary
									? t("countColumns", {
											defaultValue_one: "{{count}} column",
											defaultValue_other: "{{count}} columns",
											count: summary.columns.length,
										})
									: t("readingSchema", "reading schema…")}
							</span>
						)}
						{summary && (
							<span className="shrink-0">
								{t("countIndexed", "{{count}} indexed", {
									count: summary.indexes.length,
								})}
							</span>
						)}
					</div>
				</div>

				<div className="mt-3 grid grid-cols-3 gap-px border-y bg-border">
					<Metric label={t("rows", "Rows")} className="bg-card">
						{summary?.rows !== undefined ? (
							formatCompact(summary.rows)
						) : loading ? (
							<Shimmer className="h-3.5 w-10" />
						) : (
							"—"
						)}
					</Metric>
					<Metric label={t("onDisk", "On disk")} className="bg-card">
						{summary?.storage ? (
							formatBytes(summary.storage.total_bytes)
						) : loading ? (
							<Shimmer className="h-3.5 w-12" />
						) : (
							"—"
						)}
					</Metric>
					<Metric label={t("indexes", "Indexes")} className="bg-card">
						{summary ? (
							<span className="inline-flex items-center gap-1.5">
								{summary.indexes.length}
								<span className="inline-flex gap-0.75">
									{summary.indexes.map((index) => (
										<span
											key={index.name}
											title={`${index.index_type} · ${index.columns.join(", ")}`}
											className="size-1.25 rounded-full bg-muted-foreground"
										/>
									))}
								</span>
							</span>
						) : loading ? (
							<Shimmer className="h-3.5 w-6" />
						) : (
							"—"
						)}
					</Metric>
				</div>

				<div className="flex min-h-10.5 flex-wrap items-center gap-1.5 px-4 py-2.5">
					{chips.length > 0 ? (
						chips
					) : summary ? (
						<span className="text-[11.5px] text-muted-foreground/70">
							{t("nothingReadsThisSourceYet", "Nothing reads this source yet")}
						</span>
					) : (
						<Shimmer className="h-4 w-28" />
					)}
					<span className="ml-auto shrink-0 text-[11.5px] font-semibold text-primary">
						{t("openTable", "Open table →")}
					</span>
				</div>
			</button>

			{alert && (
				<div
					className={cn(
						"mx-4 mb-3 flex items-center gap-2 rounded-md border px-2.5 py-1.5 text-[11.5px]",
						alert.kind === "warn"
							? "border-amber-500/35 bg-amber-500/10 text-amber-600 dark:text-amber-400"
							: "border-sky-500/35 bg-sky-500/10 text-sky-600 dark:text-sky-400",
					)}
				>
					<AlertTriangle className="size-3 shrink-0" />
					<span className="flex-1">{alert.message}</span>
					<button
						type="button"
						onClick={() => onResolveAlert(entry)}
						className="shrink-0 font-semibold underline underline-offset-2"
					>
						{alert.action}
					</button>
				</div>
			)}

			<DropdownMenu>
				<DropdownMenuTrigger asChild>
					<Button
						variant="ghost"
						size="icon"
						aria-label={t("actionsForName", "Actions for {{name}}", {
							name: entry.name,
						})}
						className="absolute right-2 top-2 size-7 opacity-0 transition-opacity max-sm:opacity-100 group-hover:opacity-100 focus-visible:opacity-100 data-[state=open]:opacity-100"
					>
						<MoreVertical className="size-4" />
					</Button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="end">
					<DropdownMenuItem
						className="text-destructive focus:text-destructive"
						onSelect={(event) => {
							event.preventDefault();
							onRequestDelete();
						}}
					>
						<Trash2 className="size-4" /> {t("deleteTable", "Delete table")}
					</DropdownMenuItem>
				</DropdownMenuContent>
			</DropdownMenu>
		</Card>
	);
};

export interface SourceGridProps {
	entries: SourceEntry[];
	loading: boolean;
	searchQuery: string;
	onSelectTable: (tableName: string, userScoped?: boolean) => void;
	onRequestDelete: (entry: SourceEntry) => void;
	onResolveAlert: (entry: SourceEntry) => void;
	onCreate: () => void;
}

export const SourceGrid: React.FC<SourceGridProps> = ({
	entries,
	loading,
	searchQuery,
	onSelectTable,
	onRequestDelete,
	onResolveAlert,
	onCreate,
}) => {
	const { t } = useTranslation("settings");

	if (!entries.length && searchQuery) {
		return (
			<div className="rounded-lg border bg-card p-8 text-center">
				<Search className="mx-auto mb-4 size-10 text-muted-foreground" />
				<h3 className="mb-2 text-lg font-semibold">
					{t("noMatchesFound", "No matches found")}
				</h3>
				<p className="text-sm text-muted-foreground">
					{t("noTablesMatchQuery", 'No tables match "{{query}}".', {
						query: searchQuery,
					})}
				</p>
			</div>
		);
	}

	if (!entries.length) {
		return (
			<div className="flex flex-col items-center justify-center rounded-lg border border-dashed bg-muted/20 p-10 text-center">
				<div className="mb-4 rounded-2xl bg-primary/10 p-3 text-primary">
					<Database className="size-6" />
				</div>
				<h3 className="font-semibold">{t("noTablesYet", "No tables yet")}</h3>
				<p className="mt-1 max-w-sm text-sm text-muted-foreground">
					{t(
						"createANativeTableToStoreStructuredDataThenExploreRowsSchemaAndIndexes",
						"Create a native table to store structured data, then explore rows, schema, and indexes.",
					)}
				</p>
				<Button className="mt-5" onClick={onCreate}>
					<Plus className="size-4" /> {t("newTable", "New table")}
				</Button>
			</div>
		);
	}

	return (
		<div className="grid grid-cols-1 gap-3.5 sm:grid-cols-2 xl:grid-cols-3">
			{entries.map((entry) => (
				<SourceCard
					key={`${entry.userScoped ? "user:" : ""}${entry.name}`}
					entry={entry}
					loading={loading}
					onSelect={() => onSelectTable(entry.name, entry.userScoped)}
					onRequestDelete={() => onRequestDelete(entry)}
					onResolveAlert={onResolveAlert}
				/>
			))}
		</div>
	);
};

export interface SourceFacetBarProps {
	entries: SourceEntry[];
	active: SourceFacet;
	onChange: (facet: SourceFacet) => void;
}

export const SourceFacetBar: React.FC<SourceFacetBarProps> = ({
	entries,
	active,
	onChange,
}) => {
	const { t } = useTranslation("settings");
	const facets = useMemo(
		() =>
			(
				[
					{ id: "all", label: t("all", "All") },
					{ id: "attention", label: t("needsAttention", "Needs attention") },
					{ id: "unmapped", label: t("unmapped", "Unmapped") },
					{ id: "vector", label: t("vector", "Vector") },
					{ id: "unused", label: t("nothingReadsIt", "Nothing reads it") },
				] as const
			).map((facet) => ({
				...facet,
				count: entries.filter((entry) => matchesFacet(entry, facet.id)).length,
			})),
		[entries, t],
	);

	const handle = useCallback(
		(facet: SourceFacet) => () => onChange(facet),
		[onChange],
	);

	return (
		<div className="flex flex-wrap items-center gap-2">
			{facets.map((facet) => (
				<button
					key={facet.id}
					type="button"
					aria-pressed={active === facet.id}
					onClick={handle(facet.id)}
					disabled={facet.id !== "all" && facet.count === 0}
					className={cn(
						"inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium transition-colors",
						active === facet.id
							? "border-primary/45 bg-primary/10 text-primary"
							: "border-border bg-card text-muted-foreground hover:text-foreground",
						facet.id !== "all" && facet.count === 0 && "opacity-40",
					)}
				>
					{facet.label}
					<span className="tabular-nums opacity-75">{facet.count}</span>
				</button>
			))}
		</div>
	);
};

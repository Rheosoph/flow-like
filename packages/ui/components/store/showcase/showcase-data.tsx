"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ChevronRight,
	Database,
	Eye,
	Lock,
	Map as MapIcon,
	Network,
	Server,
	Sparkles,
	Table2,
	Terminal,
} from "lucide-react";
import { type ReactNode, useCallback, useMemo } from "react";
import type {
	IAppShowcase,
	IShowcaseTable,
} from "../../../lib/schema/app/showcase";
import {
	compact,
	familyCounts,
	familyOf,
	familyVar,
	formatNumber,
	sqlExample,
	sumRows,
	tableTitle,
} from "./model";
import {
	Chip,
	CodeBlock,
	CopyButton,
	ShowcaseSection,
	cardClass,
} from "./showcase-ui";

export const SQL_ALIAS = "my_app";

export function useRowsLabel() {
	const { t } = useTranslation("store");
	return useCallback(
		(n: number) =>
			t("showcaseRowsLabel", {
				defaultValue_one: "{{formatted}} row",
				defaultValue_other: "{{formatted}} rows",
				count: n,
				formatted: formatNumber(n),
			}),
		[t],
	);
}

export function TableChips({
	table,
	compact: hideCounts = false,
}: Readonly<{ table: IShowcaseTable; compact?: boolean }>) {
	const { t } = useTranslation("store");
	const cols = table.columns;
	const personal = cols?.filter((c) => c.personal).length ?? 0;
	return (
		<>
			{cols && !hideCounts ? (
				<Chip>
					{t("showcaseColumnCount", {
						defaultValue_one: "{{count}} column",
						defaultValue_other: "{{count}} columns",
						count: cols.length,
					})}
				</Chip>
			) : null}
			{cols?.some((c) => familyOf(c) === "vector") ? (
				<Chip icon={Sparkles}>
					{t("showcaseSearchByMeaning", "Search by meaning")}
				</Chip>
			) : null}
			{cols?.some((c) => familyOf(c) === "geo") ? (
				<Chip icon={MapIcon}>{t("showcaseMapReady", "Map-ready")}</Chip>
			) : null}
			{table.sample && table.sample.rows.length > 0 && !hideCounts ? (
				<Chip icon={Eye} tone="accent">
					{t("showcaseSampleRows", "Sample rows")}
				</Chip>
			) : null}
			{personal > 0 ? (
				<Chip icon={Lock} tone="dashed">
					{t("showcasePersonalHidden", {
						defaultValue_one: "{{count}} personal column hidden",
						defaultValue_other: "{{count}} personal columns hidden",
						count: personal,
					})}
				</Chip>
			) : null}
		</>
	);
}

function Marker({
	icon: Icon,
	label,
}: Readonly<{ icon: typeof Sparkles; label: string }>) {
	return (
		<span
			title={label}
			className="grid h-6 w-6 place-items-center rounded-md bg-muted text-muted-foreground"
		>
			<Icon className="h-3.5 w-3.5" aria-hidden="true" />
			<span className="sr-only">{label}</span>
		</span>
	);
}

const rowClass =
	"group grid w-full grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3.5 px-4 py-3 text-left transition-colors hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:outline-none sm:grid-cols-[auto_minmax(0,1fr)_7rem_7rem_6rem]";

function TableRow({
	table,
	onOpen,
}: Readonly<{ table: IShowcaseTable; onOpen: (name: string) => void }>) {
	const { t } = useTranslation("store");
	const rowsLabel = useRowsLabel();
	const cols = table.columns;
	const counts = useMemo(() => (cols ? familyCounts(cols) : []), [cols]);
	const total = counts.reduce((a, c) => a + c.n, 0);
	const personal = cols?.filter((c) => c.personal).length ?? 0;
	const color = table.object_type?.color;
	const sub = [
		table.object_type?.name,
		cols
			? t("showcaseColumnCount", {
					defaultValue_one: "{{count}} column",
					defaultValue_other: "{{count}} columns",
					count: cols.length,
				})
			: null,
	]
		.filter(Boolean)
		.join(" · ");
	return (
		<button
			type="button"
			className={rowClass}
			onClick={() => onOpen(table.name)}
		>
			<span
				className="grid h-9 w-9 place-items-center rounded-lg border border-border bg-muted/60 text-[var(--showcase-text)]"
				style={color ? { color } : undefined}
			>
				<Table2 className="h-4 w-4" aria-hidden="true" />
			</span>
			<span className="flex min-w-0 flex-col">
				<span className="truncate font-mono text-[13.5px] font-medium">
					{table.name}
				</span>
				{sub ? (
					<small className="truncate text-xs text-muted-foreground">
						{sub}
					</small>
				) : null}
			</span>
			<span
				className="hidden h-1.5 gap-px overflow-hidden rounded-full sm:flex"
				role={total > 0 ? "img" : undefined}
				aria-label={
					total > 0 ? t("showcaseColumnsByType", "Columns by type") : undefined
				}
			>
				{counts.map((c) => (
					<span
						key={c.key}
						style={{ flex: c.n, background: familyVar(c.key) }}
					/>
				))}
			</span>
			<span className="hidden items-center justify-end gap-1 sm:flex">
				{cols?.some((c) => familyOf(c) === "vector") ? (
					<Marker
						icon={Sparkles}
						label={t("showcaseSearchByMeaning", "Search by meaning")}
					/>
				) : null}
				{cols?.some((c) => familyOf(c) === "geo") ? (
					<Marker icon={MapIcon} label={t("showcaseMapReady", "Map-ready")} />
				) : null}
				{table.sample && table.sample.rows.length > 0 ? (
					<Marker icon={Eye} label={t("showcaseSampleRows", "Sample rows")} />
				) : null}
				{personal > 0 ? (
					<Marker
						icon={Lock}
						label={t("showcasePersonalHidden", {
							defaultValue_one: "{{count}} personal column hidden",
							defaultValue_other: "{{count}} personal columns hidden",
							count: personal,
						})}
					/>
				) : null}
			</span>
			<span className="flex items-center justify-end gap-2">
				{typeof table.rows === "number" ? (
					<span
						className="text-right text-sm font-semibold tabular-nums"
						title={rowsLabel(table.rows)}
					>
						{compact(table.rows)}
						<span className="ml-1 text-xs font-normal text-muted-foreground">
							{t("showcaseRowsUnit", "rows")}
						</span>
					</span>
				) : null}
				<ChevronRight
					className="h-4 w-4 text-muted-foreground transition-transform group-hover:translate-x-0.5"
					aria-hidden="true"
				/>
			</span>
		</button>
	);
}

const sourceRowClass =
	"group grid w-full grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3.5 px-4 py-3 text-left transition-colors hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:outline-none";

function GroupLabel({
	icon: Icon,
	children,
}: Readonly<{ icon: typeof Sparkles; children: ReactNode }>) {
	return (
		<div className="flex items-center gap-1.5 border-b border-border bg-muted/30 px-4 py-2 text-[11px] font-semibold uppercase tracking-widest text-muted-foreground">
			<Icon className="h-3.5 w-3.5" aria-hidden="true" />
			{children}
		</div>
	);
}

interface OntoNode {
	table: string;
	label: string;
	color: string;
	x: number;
	y: number;
	w: number;
}

function layoutOntology(showcase: IAppShowcase): {
	nodes: OntoNode[];
	byName: Map<string, OntoNode>;
} {
	const names = [
		...new Set(showcase.links.flatMap((l) => [l.from_table, l.to_table])),
	].filter((n) => showcase.tables.some((t) => t.name === n));
	const W = 640;
	const H = 250;
	const nodes = names.map((name, i) => {
		const table = showcase.tables.find((t) => t.name === name);
		const label = table ? tableTitle(table) : name;
		const angle = (i / names.length) * Math.PI * 2 - Math.PI / 2;
		const single = names.length === 1;
		return {
			table: name,
			label,
			color: table?.object_type?.color || "var(--primary)",
			x: single ? W / 2 : W / 2 + Math.cos(angle) * (W / 2 - 110),
			y: single ? H / 2 : H / 2 + Math.sin(angle) * (H / 2 - 40),
			w: Math.round(label.length * 7.6 + 46),
		};
	});
	return { nodes, byName: new Map(nodes.map((n) => [n.table, n])) };
}

function OntoChip({
	node,
	onOpen,
}: Readonly<{ node: OntoNode; onOpen: (name: string) => void }>) {
	return (
		<button
			type="button"
			onClick={() => onOpen(node.table)}
			className="inline-flex items-center gap-1.5 rounded-full border border-border py-0.5 pr-2.5 pl-2 font-medium hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
		>
			<span
				className="h-2 w-2 rounded-full"
				style={{ background: node.color }}
				aria-hidden="true"
			/>
			{node.label}
		</button>
	);
}

function OntologyCard({
	showcase,
	onOpen,
}: Readonly<{ showcase: IAppShowcase; onOpen: (name: string) => void }>) {
	const { t } = useTranslation("store");
	const { nodes, byName } = useMemo(() => layoutOntology(showcase), [showcase]);
	if (showcase.links.length === 0 || nodes.length === 0) return null;
	const links = showcase.links.filter(
		(l) => byName.has(l.from_table) && byName.has(l.to_table),
	);
	const top = Math.min(...nodes.map((n) => n.y)) - 28;
	const height = Math.max(...nodes.map((n) => n.y)) - top + 28;
	return (
		<div className={`${cardClass} space-y-3`}>
			<div>
				<h3 className="flex items-center gap-2 text-sm font-semibold">
					<Network
						className="h-4 w-4 text-muted-foreground"
						aria-hidden="true"
					/>
					{t("showcaseObjectTypes", "Object types")}
				</h3>
				<p className="text-xs text-muted-foreground">
					{t(
						"showcaseObjectTypesHint",
						"How the tables relate. Select an object to open its table.",
					)}
				</p>
			</div>
			<ul className="flex flex-col gap-2 sm:hidden">
				{links.map((l) => {
					const a = byName.get(l.from_table) as OntoNode;
					const b = byName.get(l.to_table) as OntoNode;
					return (
						<li
							key={`${l.from_table}>${l.to_table}>${l.label}`}
							className="flex flex-wrap items-center gap-1.5 text-[13px]"
						>
							<OntoChip node={a} onOpen={onOpen} />
							<span className="text-xs text-muted-foreground">{l.label} →</span>
							<OntoChip node={b} onOpen={onOpen} />
						</li>
					);
				})}
			</ul>
			<div className="hidden overflow-x-auto sm:block">
				<svg
					viewBox={`0 ${top} 640 ${height}`}
					className="mx-auto block h-auto w-full max-w-160 min-w-130"
					// biome-ignore lint/a11y/useSemanticElements: svg has no semantic group element
					role="group"
					aria-label={t(
						"showcaseObjectTypesLabel",
						"Object types and how they link",
					)}
				>
					{links.map((l) => {
						const a = byName.get(l.from_table) as OntoNode;
						const b = byName.get(l.to_table) as OntoNode;
						const mx = (a.x + b.x) / 2;
						const my = (a.y + b.y) / 2;
						const w = l.label.length * 6.4 + 16;
						return (
							<g key={`${l.from_table}>${l.to_table}>${l.label}`}>
								<line
									x1={a.x}
									y1={a.y}
									x2={b.x}
									y2={b.y}
									className="stroke-border"
									strokeWidth={1.5}
								/>
								<rect
									x={mx - w / 2}
									y={my - 10}
									width={w}
									height={20}
									rx={10}
									className="fill-card stroke-border"
								/>
								<text
									x={mx}
									y={my + 4}
									textAnchor="middle"
									className="fill-muted-foreground text-[11px] font-medium"
								>
									{l.label}
								</text>
							</g>
						);
					})}
					{nodes.map((n) => {
						const x = n.x - n.w / 2;
						return (
							<g
								key={n.table}
								// biome-ignore lint/a11y/useSemanticElements: svg group acting as a button
								role="button"
								tabIndex={0}
								aria-label={t("showcaseOpenTable", "Open {{name}}", {
									name: n.label,
								})}
								className="group cursor-pointer outline-none"
								onClick={() => onOpen(n.table)}
								onKeyDown={(e) => {
									if (e.key === "Enter" || e.key === " ") {
										e.preventDefault();
										onOpen(n.table);
									}
								}}
							>
								<rect
									x={x}
									y={n.y - 17}
									width={n.w}
									height={34}
									rx={17}
									className="fill-card stroke-border group-hover:fill-muted group-focus-visible:stroke-ring"
									strokeWidth={1.5}
								/>
								<circle cx={x + 18} cy={n.y} r={5} style={{ fill: n.color }} />
								<text
									x={x + 30}
									y={n.y + 4.5}
									className="fill-foreground text-[12.5px] font-semibold"
								>
									{n.label}
								</text>
							</g>
						);
					})}
				</svg>
			</div>
		</div>
	);
}

function QueryCard({
	example,
}: Readonly<{ example: { table: string; sql: string } }>) {
	const { t } = useTranslation("store");
	const getText = useCallback(() => example.sql, [example.sql]);
	return (
		<div className="space-y-3">
			<div>
				<h3 className="flex items-center gap-2 text-sm font-semibold">
					<Terminal
						className="h-4 w-4 text-muted-foreground"
						aria-hidden="true"
					/>
					{t("showcaseQueryTitle", "Query it from your own apps")}
				</h3>
				<p className="mt-1 max-w-prose text-xs text-muted-foreground">
					{t(
						"showcaseQueryHint",
						"Members can read this data with SQL from Home widgets and their own apps. Replace {{alias}} with the name you give the app.",
						{ alias: SQL_ALIAS },
					)}
				</p>
			</div>
			<CodeBlock
				code={example.sql}
				header={
					<>
						<span className="font-mono text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
							SQL
						</span>
						<CopyButton getText={getText} variant="ghost" />
					</>
				}
			/>
		</div>
	);
}

export function ShowcaseData({
	showcase,
	onOpen,
}: Readonly<{
	showcase: IAppShowcase;
	onOpen: (kind: "table" | "source", id: string) => void;
}>) {
	const { t } = useTranslation("store");
	const rowsLabel = useRowsLabel();
	const total = sumRows(showcase.tables);
	const example = useMemo(() => sqlExample(showcase, SQL_ALIAS), [showcase]);
	const openTable = useCallback((n: string) => onOpen("table", n), [onOpen]);

	const subtitle = [
		showcase.tables.length > 0
			? t("showcaseTableCount", {
					defaultValue_one: "{{count}} table",
					defaultValue_other: "{{count}} tables",
					count: showcase.tables.length,
				})
			: "",
		total !== null ? rowsLabel(total) : "",
	]
		.filter(Boolean)
		.join(" · ");

	return (
		<ShowcaseSection
			id="data"
			icon={Database}
			eyebrow={t("showcaseDataEyebrow", "Data")}
			title={t("showcaseDataTitle", "What it keeps")}
			subtitle={subtitle || undefined}
		>
			{showcase.tables.length > 0 || showcase.sources.length > 0 ? (
				<div className="overflow-hidden rounded-xl border border-border bg-card">
					{showcase.tables.length > 0 && showcase.sources.length > 0 ? (
						<GroupLabel icon={Table2}>
							{t("showcaseTablesGroup", "Tables")}
						</GroupLabel>
					) : null}
					<div className="divide-y divide-border">
						{showcase.tables.map((tb) => (
							<TableRow key={tb.name} table={tb} onOpen={openTable} />
						))}
					</div>
					{showcase.sources.length > 0 ? (
						<>
							<GroupLabel icon={Server}>
								{t("showcaseLiveConnections", "Live connections")}
							</GroupLabel>
							<div className="divide-y divide-border">
								{showcase.sources.map((s) => (
									<button
										key={s.id}
										type="button"
										className={sourceRowClass}
										onClick={() => onOpen("source", s.id)}
									>
										<span className="grid h-9 w-9 place-items-center rounded-lg border border-border bg-muted/60 text-muted-foreground">
											<Server className="h-4 w-4" aria-hidden="true" />
										</span>
										<span className="flex min-w-0 flex-col">
											<span className="truncate font-mono text-[13.5px] font-medium">
												{s.name}
											</span>
											<small className="truncate text-xs text-muted-foreground">
												{s.driver}
											</small>
										</span>
										<ChevronRight
											className="h-4 w-4 text-muted-foreground transition-transform group-hover:translate-x-0.5"
											aria-hidden="true"
										/>
									</button>
								))}
							</div>
						</>
					) : null}
				</div>
			) : null}
			<OntologyCard showcase={showcase} onOpen={openTable} />
			{example ? <QueryCard example={example} /> : null}
		</ShowcaseSection>
	);
}

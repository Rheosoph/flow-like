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
import { useCallback, useMemo } from "react";
import type {
	IAppShowcase,
	IShowcaseTable,
} from "../../../lib/schema/app/showcase";
import {
	compact,
	familyCounts,
	familyOf,
	formatNumber,
	sqlExample,
	sumRows,
	tableTitle,
} from "./model";
import {
	Chip,
	CodeBlock,
	CompositionBar,
	CopyButton,
	ShowcaseSection,
	cardClass,
	familyParts,
	interactiveCardClass,
	useFamilyLabels,
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

export function TableChips({ table }: Readonly<{ table: IShowcaseTable }>) {
	const { t } = useTranslation("store");
	const cols = table.columns;
	const personal = cols?.filter((c) => c.personal).length ?? 0;
	return (
		<>
			{cols ? (
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
			{table.sample && table.sample.rows.length > 0 ? (
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

function TableCard({
	table,
	onOpen,
}: Readonly<{ table: IShowcaseTable; onOpen: (name: string) => void }>) {
	const { t } = useTranslation("store");
	const labels = useFamilyLabels();
	const rowsLabel = useRowsLabel();
	const parts = useMemo(
		() =>
			table.columns ? familyParts(familyCounts(table.columns), labels) : [],
		[table.columns, labels],
	);
	return (
		<button
			type="button"
			className={`${interactiveCardClass} flex flex-col gap-3`}
			onClick={() => onOpen(table.name)}
		>
			<span className="flex items-start gap-2.5">
				<span className="grid h-8 w-8 shrink-0 place-items-center rounded-lg bg-muted text-[var(--showcase-text)]">
					<Table2 className="h-4 w-4" aria-hidden="true" />
				</span>
				<span className="flex min-w-0 flex-1 flex-col">
					<span className="wrap-anywhere font-mono text-[13.5px] font-medium">
						{table.name}
					</span>
					{table.object_type ? (
						<small className="text-xs text-muted-foreground">
							{tableTitle(table)}
						</small>
					) : null}
				</span>
				{typeof table.rows === "number" ? (
					<span className="flex flex-col text-right">
						<b className="text-[17px] font-semibold tabular-nums tracking-tight">
							{compact(table.rows)}
						</b>
						<small
							className="text-[11px] text-muted-foreground"
							title={rowsLabel(table.rows)}
						>
							{t("showcaseRowsUnit", "rows")}
						</small>
					</span>
				) : null}
			</span>
			{parts.length > 0 ? (
				<CompositionBar
					parts={parts}
					label={t("showcaseColumnsByType", "Columns by type")}
				/>
			) : null}
			<span className="flex flex-wrap gap-1.5">
				<TableChips table={table} />
			</span>
		</button>
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
			<div className="overflow-x-auto">
				<svg
					viewBox="0 0 640 250"
					className="block h-auto w-full min-w-130"
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
		<div className={`${cardClass} space-y-3`}>
			<div className="flex flex-wrap items-start justify-between gap-3">
				<div>
					<h3 className="flex items-center gap-2 text-sm font-semibold">
						<Terminal
							className="h-4 w-4 text-muted-foreground"
							aria-hidden="true"
						/>
						{t("showcaseQueryTitle", "Query it from your own apps")}
					</h3>
					<p className="text-xs text-muted-foreground">
						{t(
							"showcaseQueryHint",
							"Members can read this data with SQL from Home widgets and their own apps. Replace {{alias}} with the name you give the app.",
							{ alias: SQL_ALIAS },
						)}
					</p>
				</div>
				<CopyButton getText={getText} />
			</div>
			<CodeBlock code={example.sql} />
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
			{showcase.tables.length > 0 ? (
				<div className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,300px),1fr))] gap-3">
					{showcase.tables.map((tb) => (
						<TableCard key={tb.name} table={tb} onOpen={openTable} />
					))}
				</div>
			) : null}
			<OntologyCard showcase={showcase} onOpen={openTable} />
			{showcase.sources.length > 0 ? (
				<div className="space-y-2.5">
					<h3 className="flex items-center gap-2 text-sm font-semibold">
						<Server
							className="h-4 w-4 text-muted-foreground"
							aria-hidden="true"
						/>
						{t("showcaseLiveConnections", "Live connections")}
					</h3>
					<div className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,280px),1fr))] gap-2.5">
						{showcase.sources.map((s) => (
							<button
								key={s.id}
								type="button"
								className={`${interactiveCardClass} flex items-center gap-2.5 py-3`}
								onClick={() => onOpen("source", s.id)}
							>
								<Server
									className="h-4 w-4 shrink-0 text-muted-foreground"
									aria-hidden="true"
								/>
								<span className="flex min-w-0 flex-1 flex-col">
									<b className="font-mono text-[13px] font-medium">{s.name}</b>
									<small className="text-xs text-muted-foreground">
										{s.driver}
									</small>
								</span>
								<ChevronRight
									className="h-4 w-4 text-muted-foreground"
									aria-hidden="true"
								/>
							</button>
						))}
					</div>
				</div>
			) : null}
			{example ? <QueryCard example={example} /> : null}
		</ShowcaseSection>
	);
}

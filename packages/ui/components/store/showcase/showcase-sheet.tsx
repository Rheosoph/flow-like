"use client";

import { useTranslation } from "@flow-like/locales";
import { Info, Lock, ShieldCheck } from "lucide-react";
import { type ReactNode, useCallback, useMemo, useState } from "react";
import type {
	IAppShowcase,
	IShowcaseColumn,
	IShowcaseEvent,
	IShowcaseFlow,
	IShowcasePage,
	IShowcaseSource,
	IShowcaseTable,
	IShowcaseWidget,
} from "../../../lib/schema/app/showcase";
import { cn } from "../../../lib/utils";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
} from "../../ui/sheet";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
import {
	type ShowcaseGraph,
	type ShowcaseNodeKind,
	type SnippetLang,
	endpointMethod,
	endpointSnippet,
	familyCounts,
	familyOf,
	familyVar,
	formatNumber,
	isEndpointEvent,
	nodeIcon,
	nodeKey,
	tableTitle,
} from "./model";
import { TableChips, useRowsLabel } from "./showcase-data";
import { useNodeSub } from "./showcase-map";
import {
	AccessChip,
	EventTechnical,
	MethodBadge,
	type ShowcaseAccess,
	useEventKindLabel,
} from "./showcase-triggers";
import {
	CodeBlock,
	CompositionBar,
	CopyButton,
	LockedNote,
	NODE_TONE,
	familyParts,
	useFamilyLabels,
} from "./showcase-ui";
import { Wireframe } from "./wireframe";

export interface ShowcaseSelection {
	kind: ShowcaseNodeKind;
	id: string;
}

const SCORE_KEYS = [
	"security",
	"privacy",
	"performance",
	"governance",
	"reliability",
	"cost",
] as const;

function Related({
	graph,
	selection,
	onSelect,
}: Readonly<{
	graph: ShowcaseGraph;
	selection: ShowcaseSelection;
	onSelect: (s: ShowcaseSelection) => void;
}>) {
	const { t } = useTranslation("store");
	const key = nodeKey(selection.kind, selection.id);
	const up = graph.edges.filter((e) => e.to === key).map((e) => e.from);
	const down = graph.edges.filter((e) => e.from === key).map((e) => e.to);
	if (up.length === 0 && down.length === 0) return null;

	const row = (title: string, keys: string[]) =>
		keys.length === 0 ? null : (
			<div className="flex flex-wrap items-center gap-1.5">
				<small className="w-full text-xs text-muted-foreground">{title}</small>
				{keys.map((k) => {
					const n = graph.nodes.get(k);
					if (!n) return null;
					const Icon = nodeIcon(n.kind, n.eventType);
					return (
						<button
							key={k}
							type="button"
							onClick={() => onSelect({ kind: n.kind, id: n.id })}
							className="inline-flex items-center gap-1.5 rounded-full border border-border bg-card py-1 pr-2.5 pl-1.5 text-[12.5px] hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
						>
							<Icon
								className={cn("h-3.5 w-3.5", NODE_TONE[n.kind])}
								aria-hidden="true"
							/>
							<span className={cn(n.mono && "font-mono")}>{n.label}</span>
						</button>
					);
				})}
			</div>
		);

	return (
		<div className="space-y-3">
			<h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
				{t("showcaseConnected", "Connected")}
			</h4>
			{row(t("showcaseComesFrom", "Comes from"), up)}
			{row(t("showcaseLeadsTo", "Leads to"), down)}
		</div>
	);
}

function Stats({ items }: Readonly<{ items: [string, string][] }>) {
	return (
		<div className="grid grid-cols-[repeat(auto-fit,minmax(100px,1fr))] gap-2.5">
			{items.map(([k, v]) => (
				<div key={k} className="flex flex-col">
					<small className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
						{k}
					</small>
					<b className="text-base font-semibold tabular-nums">{v}</b>
				</div>
			))}
		</div>
	);
}

function Caption({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<p className="flex items-start gap-1.5 text-xs text-muted-foreground">
			<Info className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
			{children}
		</p>
	);
}

function cell(value: unknown): string {
	if (value === null || value === undefined) return "—";
	if (typeof value === "object") return JSON.stringify(value);
	return String(value);
}

function ColumnList({ columns }: Readonly<{ columns: IShowcaseColumn[] }>) {
	const { t } = useTranslation("store");
	const labels = useFamilyLabels();
	const parts = useMemo(
		() => familyParts(familyCounts(columns), labels),
		[columns, labels],
	);
	return (
		<div className="space-y-3">
			<CompositionBar
				parts={parts}
				label={t("showcaseColumnsByType", "Columns by type")}
			/>
			<ul className="rounded-md border border-border">
				{columns.map((c) => (
					<li
						key={c.name}
						className="grid grid-cols-[10px_minmax(0,1fr)_auto] items-center gap-x-2.5 gap-y-0.5 border-b border-border px-3 py-2 last:border-b-0"
					>
						<span
							className="row-span-2 h-2 w-2 rounded-xs"
							style={{ background: familyVar(familyOf(c)) }}
							aria-hidden="true"
						/>
						<span className="wrap-anywhere font-mono text-[13px] font-medium">
							{c.name}
						</span>
						<span className="row-span-2 text-[11.5px] text-muted-foreground">
							{c.personal ? (
								<span className="inline-flex items-center gap-1 rounded-full border border-dashed border-border px-2 py-0.5">
									<Lock className="h-3 w-3" aria-hidden="true" />
									{t("showcaseNeverShown", "Never shown")}
								</span>
							) : (
								labels[familyOf(c)]
							)}
						</span>
						<span className="col-start-2 font-mono text-[11.5px] text-muted-foreground">
							{c.data_type}
						</span>
					</li>
				))}
			</ul>
		</div>
	);
}

function SampleTable({ table }: Readonly<{ table: IShowcaseTable }>) {
	const { t } = useTranslation("store");
	const columns = table.columns ?? [];
	const rows = table.sample?.rows ?? [];
	return (
		<div className="space-y-3">
			<div className="overflow-x-auto rounded-md border border-border">
				<table className="min-w-full border-collapse text-xs">
					<thead>
						<tr>
							{columns.map((c) => (
								<th
									key={c.name}
									className="whitespace-nowrap border-b border-border bg-muted px-2.5 py-2 text-left font-mono text-[11px] font-medium text-muted-foreground"
								>
									{c.name}
								</th>
							))}
						</tr>
					</thead>
					<tbody>
						{rows.map((r, ri) => (
							// biome-ignore lint/suspicious/noArrayIndexKey: rows have no ids
							<tr key={ri}>
								{columns.map((c, ci) =>
									c.personal ? (
										<td
											key={c.name}
											data-masked
											className="whitespace-nowrap border-b border-border bg-muted/40 px-2.5 py-1.5 font-mono text-muted-foreground"
										>
											<Lock
												className="mr-1 inline h-3 w-3 align-[-1px]"
												aria-hidden="true"
											/>
											{t("showcaseHidden", "hidden")}
										</td>
									) : (
										<td
											key={c.name}
											className="whitespace-nowrap border-b border-border px-2.5 py-1.5 font-mono tabular-nums"
										>
											{cell(r[ci])}
										</td>
									),
								)}
							</tr>
						))}
					</tbody>
				</table>
			</div>
			<Caption>
				{t(
					"showcaseSampleCaption",
					"Rows picked by the publisher. Personal columns are always hidden.",
				)}
			</Caption>
		</div>
	);
}

function TableSheet({
	table,
	graph,
	onSelect,
}: Readonly<{
	table: IShowcaseTable;
	graph: ShowcaseGraph;
	onSelect: (s: ShowcaseSelection) => void;
}>) {
	const { t } = useTranslation("store");
	const rowsLabel = useRowsLabel();
	const stats: [string, string][] = [];
	if (typeof table.rows === "number")
		stats.push([t("showcaseStatRowsLabel", "Rows"), formatNumber(table.rows)]);
	if (table.columns)
		stats.push([
			t("showcaseStatColumnsLabel", "Columns"),
			String(table.columns.length),
		]);
	const hasSample = !!table.sample && table.sample.rows.length > 0;
	return (
		<div className="space-y-4">
			{stats.length > 0 ? <Stats items={stats} /> : null}
			<div className="flex flex-wrap gap-1.5">
				<TableChips table={table} />
			</div>
			<Tabs defaultValue="columns">
				<TabsList>
					<TabsTrigger value="columns">
						{t("showcaseTabColumns", "Columns")}
					</TabsTrigger>
					<TabsTrigger value="sample">
						{t("showcaseTabSample", "Sample rows")}
					</TabsTrigger>
					<TabsTrigger value="indexes">
						{t("showcaseTabIndexes", "Indexes")}
					</TabsTrigger>
					<TabsTrigger value="used">
						{t("showcaseTabUsedBy", "Used by")}
					</TabsTrigger>
				</TabsList>
				<TabsContent value="columns" className="pt-3">
					{table.columns ? (
						<ColumnList columns={table.columns} />
					) : (
						<LockedNote icon={Lock}>
							{t(
								"showcaseLockedColumns",
								"The publisher shares this table's name only. Join to see its columns.",
							)}
						</LockedNote>
					)}
				</TabsContent>
				<TabsContent value="sample" className="pt-3">
					{hasSample ? (
						<SampleTable table={table} />
					) : (
						<LockedNote icon={Lock}>
							{t(
								"showcaseLockedSample",
								"The publisher doesn't share sample rows for this table. Join to explore it.",
							)}
						</LockedNote>
					)}
				</TabsContent>
				<TabsContent value="indexes" className="pt-3">
					{table.indexes && table.indexes.length > 0 ? (
						<ul className="rounded-md border border-border">
							{table.indexes.map((ix) => (
								<li
									key={ix.name}
									className="flex flex-col gap-0.5 border-b border-border px-3 py-2 last:border-b-0"
								>
									<span className="flex items-center justify-between gap-2">
										<span className="font-mono text-[13px] font-medium">
											{ix.name}
										</span>
										<span className="text-[11.5px] text-muted-foreground">
											{ix.index_type}
										</span>
									</span>
									<span className="font-mono text-[11.5px] text-muted-foreground">
										{ix.columns.join(", ")}
									</span>
								</li>
							))}
						</ul>
					) : (
						<LockedNote icon={Lock}>
							{t(
								"showcaseLockedIndexes",
								"Join to see how this table is indexed.",
							)}
						</LockedNote>
					)}
				</TabsContent>
				<TabsContent value="used" className="space-y-3 pt-3">
					{table.object_type ? (
						<dl className="grid grid-cols-[110px_minmax(0,1fr)] gap-x-3.5 gap-y-2 text-[13px]">
							<dt className="text-muted-foreground">
								{t("showcaseObjectType", "Object type")}
							</dt>
							<dd>{table.object_type.name}</dd>
						</dl>
					) : null}
					<Related
						graph={graph}
						selection={{ kind: "table", id: table.name }}
						onSelect={onSelect}
					/>
					{graph.edges.every((e) => e.to !== nodeKey("table", table.name)) &&
					!table.object_type ? (
						<p className="text-xs text-muted-foreground">
							{t("showcaseNothingUsesIt", "Nothing else on this page uses it.")}
						</p>
					) : null}
				</TabsContent>
			</Tabs>
			<span className="sr-only">
				{typeof table.rows === "number" ? rowsLabel(table.rows) : ""}
			</span>
		</div>
	);
}

function EventSheet({
	event,
	graph,
	onSelect,
}: Readonly<{
	event: IShowcaseEvent;
	graph: ShowcaseGraph;
	onSelect: (s: ShowcaseSelection) => void;
}>) {
	const { t } = useTranslation("store");
	const [lang, setLang] = useState<SnippetLang>("curl");
	const snippet = isEndpointEvent(event) ? endpointSnippet(event, lang) : null;
	const getText = useCallback(() => snippet ?? "", [snippet]);
	return (
		<div className="space-y-4">
			{event.description ? (
				<p className="text-sm">{event.description}</p>
			) : null}
			{event.technical ? (
				<div className="space-y-2">
					<h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
						{t("showcaseTechnical", "Technical details")}
					</h4>
					{isEndpointEvent(event) ? (
						<div className="flex flex-wrap items-center gap-2 rounded-md border border-border bg-muted/40 px-3.5 py-3 text-[15px]">
							<MethodBadge method={endpointMethod(event)} />
							<span className="wrap-anywhere font-mono">
								{event.technical.path}
							</span>
						</div>
					) : (
						<EventTechnical event={event} />
					)}
				</div>
			) : (
				<p className="text-sm text-muted-foreground">
					{t(
						"showcaseEventNoTechnical",
						"The publisher shares that this trigger exists. Its details open after you join.",
					)}
				</p>
			)}
			{snippet ? (
				<div className="space-y-2.5">
					<div className="flex flex-wrap items-center justify-between gap-2">
						<Tabs value={lang} onValueChange={(v) => setLang(v as SnippetLang)}>
							<TabsList>
								<TabsTrigger value="curl">curl</TabsTrigger>
								<TabsTrigger value="js">JavaScript</TabsTrigger>
								<TabsTrigger value="py">Python</TabsTrigger>
							</TabsList>
						</Tabs>
						<CopyButton getText={getText} />
					</div>
					<CodeBlock code={snippet} />
					<Caption>
						{t(
							"showcaseEndpointNote",
							"The endpoint address and your token appear after you join the app.",
						)}
					</Caption>
				</div>
			) : null}
			<Related
				graph={graph}
				selection={{ kind: "event", id: event.id }}
				onSelect={onSelect}
			/>
		</div>
	);
}

function PageSheet({
	page,
	graph,
	onSelect,
}: Readonly<{
	page: IShowcasePage;
	graph: ShowcaseGraph;
	onSelect: (s: ShowcaseSelection) => void;
}>) {
	const { t } = useTranslation("store");
	return (
		<div className="space-y-4">
			{page.outline ? (
				<Wireframe
					outline={page.outline}
					large
					label={t("showcasePagePreview", "Layout preview of {{name}}", {
						name: page.name,
					})}
				/>
			) : null}
			{page.description ? <p className="text-sm">{page.description}</p> : null}
			<dl className="grid grid-cols-[110px_minmax(0,1fr)] gap-x-3.5 gap-y-2 text-[13px]">
				{page.route ? (
					<>
						<dt className="text-muted-foreground">
							{t("showcaseRoute", "Route")}
						</dt>
						<dd className="font-mono">{page.route}</dd>
					</>
				) : null}
				{typeof page.component_count === "number" ? (
					<>
						<dt className="text-muted-foreground">
							{t("showcaseBuiltFrom", "Built from")}
						</dt>
						<dd>
							{t("showcaseComponentCount", {
								defaultValue_one: "{{count}} component",
								defaultValue_other: "{{count}} components",
								count: page.component_count,
							})}
						</dd>
					</>
				) : null}
			</dl>
			<Related
				graph={graph}
				selection={{ kind: "page", id: page.id }}
				onSelect={onSelect}
			/>
		</div>
	);
}

function FlowSheet({
	flow,
	graph,
	onSelect,
}: Readonly<{
	flow: IShowcaseFlow;
	graph: ShowcaseGraph;
	onSelect: (s: ShowcaseSelection) => void;
}>) {
	const { t } = useTranslation("store");
	const labels: Record<(typeof SCORE_KEYS)[number], string> = {
		security: t("showcaseScoreSecurity", "Security"),
		privacy: t("showcaseScorePrivacy", "Privacy"),
		performance: t("showcaseScorePerformance", "Performance"),
		governance: t("showcaseScoreGovernance", "Governance"),
		reliability: t("showcaseScoreReliability", "Reliability"),
		cost: t("showcaseScoreCost", "Cost"),
	};
	return (
		<div className="space-y-4">
			{flow.description ? <p className="text-sm">{flow.description}</p> : null}
			<Stats
				items={[[t("showcaseStatNodes", "Nodes"), String(flow.node_count)]]}
			/>
			{flow.scores ? (
				<div className="space-y-2">
					<h4 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
						{t("showcaseQuality", "Quality")}
					</h4>
					<ul className="space-y-1.5">
						{SCORE_KEYS.map((k) => {
							const v = flow.scores?.[k] ?? 0;
							return (
								<li
									key={k}
									className="grid grid-cols-[96px_1fr_28px] items-center gap-2 text-[12.5px]"
								>
									<span>{labels[k]}</span>
									<span
										className="h-1.5 overflow-hidden rounded-xs bg-muted"
										role="img"
										aria-label={`${v}/10`}
									>
										<span
											className="block h-full rounded-r-xs bg-primary"
											style={{
												width: `${Math.min(100, Math.max(0, v * 10))}%`,
											}}
										/>
									</span>
									<span className="text-right font-semibold tabular-nums">
										{v}
									</span>
								</li>
							);
						})}
					</ul>
				</div>
			) : null}
			<Related
				graph={graph}
				selection={{ kind: "flow", id: flow.id }}
				onSelect={onSelect}
			/>
		</div>
	);
}

function WidgetSheet({
	widget,
	graph,
	onSelect,
}: Readonly<{
	widget: IShowcaseWidget;
	graph: ShowcaseGraph;
	onSelect: (s: ShowcaseSelection) => void;
}>) {
	return (
		<div className="space-y-4">
			{widget.thumbnail ? (
				<img
					src={widget.thumbnail}
					alt=""
					className="aspect-video w-full rounded-lg border border-border object-cover"
				/>
			) : null}
			{widget.description ? (
				<p className="text-sm">{widget.description}</p>
			) : null}
			<Related
				graph={graph}
				selection={{ kind: "widget", id: widget.id }}
				onSelect={onSelect}
			/>
		</div>
	);
}

function SourceSheet({ source }: Readonly<{ source: IShowcaseSource }>) {
	const { t } = useTranslation("store");
	return (
		<div className="space-y-4">
			<dl className="grid grid-cols-[110px_minmax(0,1fr)] gap-x-3.5 gap-y-2 text-[13px]">
				<dt className="text-muted-foreground">
					{t("showcaseDriver", "Driver")}
				</dt>
				<dd>{source.driver}</dd>
			</dl>
			<Caption>
				{t(
					"showcaseSourceNote",
					"Hosts and credentials stay with the publisher. Visitors only see the name and driver.",
				)}
			</Caption>
		</div>
	);
}

interface SheetModel {
	title: string;
	mono: boolean;
	kicker: string;
	badge?: ReactNode;
	body: ReactNode;
}

export function ShowcaseSheet({
	showcase,
	graph,
	access,
	selection,
	onSelect,
	onClose,
}: Readonly<{
	showcase: IAppShowcase;
	graph: ShowcaseGraph;
	access: ShowcaseAccess;
	selection: ShowcaseSelection | null;
	onSelect: (s: ShowcaseSelection) => void;
	onClose: () => void;
}>) {
	const { t } = useTranslation("store");
	const eventLabel = useEventKindLabel();
	const nodeSub = useNodeSub();

	const model = useMemo((): SheetModel | null => {
		if (!selection) return null;
		const { kind, id } = selection;
		const sub = (k: ShowcaseNodeKind, fallback: string) => {
			const n = graph.nodes.get(nodeKey(k, id));
			return n ? nodeSub(n) : fallback;
		};
		if (kind === "table") {
			const tb = showcase.tables.find((x) => x.name === id);
			return tb
				? {
						title: tb.name,
						mono: true,
						kicker: t("showcaseKickerTable", "Table · {{title}}", {
							title: tableTitle(tb),
						}),
						body: <TableSheet table={tb} graph={graph} onSelect={onSelect} />,
					}
				: null;
		}
		if (kind === "event") {
			const ev = showcase.events.find((x) => x.id === id);
			return ev
				? {
						title: ev.name,
						mono: false,
						kicker: eventLabel(ev.event_type),
						badge: <AccessChip access={access} />,
						body: <EventSheet event={ev} graph={graph} onSelect={onSelect} />,
					}
				: null;
		}
		if (kind === "page") {
			const p = showcase.pages.find((x) => x.id === id);
			return p
				? {
						title: p.name,
						mono: false,
						kicker: t("showcasePage", "Page"),
						body: <PageSheet page={p} graph={graph} onSelect={onSelect} />,
					}
				: null;
		}
		if (kind === "flow") {
			const f = showcase.flows.find((x) => x.id === id);
			return f
				? {
						title: f.name,
						mono: false,
						kicker: t("showcaseKickerFlow", "Flow"),
						body: <FlowSheet flow={f} graph={graph} onSelect={onSelect} />,
					}
				: null;
		}
		if (kind === "widget") {
			const w = showcase.widgets.find((x) => x.id === id);
			return w
				? {
						title: w.name,
						mono: false,
						kicker: sub("widget", t("showcaseHomeWidget", "Home widget")),
						body: <WidgetSheet widget={w} graph={graph} onSelect={onSelect} />,
					}
				: null;
		}
		const s = showcase.sources.find((x) => x.id === id);
		return s
			? {
					title: s.name,
					mono: true,
					kicker: t("showcaseKickerConnection", "Connection · {{driver}}", {
						driver: s.driver,
					}),
					body: <SourceSheet source={s} />,
				}
			: null;
	}, [selection, showcase, graph, access, onSelect, t, eventLabel, nodeSub]);

	const Icon = selection
		? nodeIcon(
				selection.kind,
				showcase.events.find((e) => e.id === selection.id)?.event_type,
			)
		: ShieldCheck;

	return (
		<Sheet open={!!model} onOpenChange={(o) => !o && onClose()}>
			<SheetContent
				side="right"
				className="w-full gap-0 overflow-hidden sm:max-w-xl"
			>
				{model ? (
					<>
						<SheetHeader className="border-b border-border pr-12">
							<div className="flex items-start gap-3">
								<span
									className={cn(
										"grid h-9.5 w-9.5 shrink-0 place-items-center rounded-lg bg-muted",
										selection ? NODE_TONE[selection.kind] : "",
									)}
								>
									<Icon className="h-4.5 w-4.5" aria-hidden="true" />
								</span>
								<div className="flex min-w-0 flex-1 flex-col gap-1">
									<span className="text-[11px] font-semibold uppercase tracking-widest text-muted-foreground">
										{model.kicker}
									</span>
									<SheetTitle
										className={cn(
											"wrap-anywhere text-lg",
											model.mono && "font-mono text-base font-medium",
										)}
									>
										{model.title}
									</SheetTitle>
									<SheetDescription className="sr-only">
										{model.kicker}
									</SheetDescription>
									{model.badge ? <div>{model.badge}</div> : null}
								</div>
							</div>
						</SheetHeader>
						<div
							key={`${selection?.kind}:${selection?.id}`}
							className="flex-1 overflow-y-auto overscroll-contain px-4 pb-8"
						>
							{model.body}
						</div>
					</>
				) : null}
			</SheetContent>
		</Sheet>
	);
}

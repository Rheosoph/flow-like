"use client";

import { useTranslation } from "@flow-like/locales";
import { Info, Lock, ShieldCheck } from "lucide-react";
import { type ReactNode, useCallback, useMemo, useState } from "react";
import { useIsMobile } from "../../../hooks/use-mobile";
import type {
	IAppShowcase,
	IShowcaseColumn,
	IShowcaseEvent,
	IShowcaseFlow,
	IShowcaseIndex,
	IShowcasePage,
	IShowcaseSource,
	IShowcaseTable,
	IShowcaseWidget,
} from "../../../lib/schema/app/showcase";
import { cn } from "../../../lib/utils";
import type { QueryColumn } from "../../../state/backend-state/query-state";
import {
	QueryResultTable,
	type ResultTablePresentation,
} from "../../settings/data-studio/query-workbench/query-result-table";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogTitle,
} from "../../ui/dialog";
import { Sheet, SheetContent } from "../../ui/sheet";
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
import { TableChips } from "./showcase-data";
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
	TabCount,
	familyParts,
	gridCellClass,
	gridHeadClass,
	gridTableClass,
	gridWrapClass,
	tabsListClass,
	tabsTriggerClass,
	useFamilyLabels,
} from "./showcase-ui";
import { PageFrame, Wireframe } from "./wireframe";

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

function SectionLabel({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<h4 className="text-[11px] font-semibold uppercase tracking-widest text-muted-foreground">
			{children}
		</h4>
	);
}

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
			<div className="grid gap-2 sm:grid-cols-[110px_minmax(0,1fr)] sm:items-baseline">
				<small className="text-xs text-muted-foreground">{title}</small>
				<div className="flex flex-wrap gap-1.5">
					{keys.map((k) => {
						const n = graph.nodes.get(k);
						if (!n) return null;
						const Icon = nodeIcon(n.kind, n.eventType);
						return (
							<button
								key={k}
								type="button"
								onClick={() => onSelect({ kind: n.kind, id: n.id })}
								className="inline-flex items-center gap-1.5 rounded-md border border-border bg-card py-1 pr-2.5 pl-1.5 text-[12.5px] transition-colors hover:border-foreground/25 hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
							>
								<Icon
									className={cn("h-3.5 w-3.5", NODE_TONE[n.kind])}
									aria-hidden="true"
								/>
								<span className={cn(n.mono && "font-mono text-xs")}>
									{n.label}
								</span>
							</button>
						);
					})}
				</div>
			</div>
		);

	return (
		<div className="space-y-3">
			<SectionLabel>{t("showcaseConnected", "Connected")}</SectionLabel>
			{row(t("showcaseComesFrom", "Comes from"), up)}
			{row(t("showcaseLeadsTo", "Leads to"), down)}
		</div>
	);
}

function Facts({ items }: Readonly<{ items: [string, string][] }>) {
	return (
		<dl className="flex flex-wrap gap-x-6 gap-y-2">
			{items.map(([label, value]) => (
				<div key={label} className="flex flex-col">
					<dt className="text-[11px] font-medium uppercase tracking-wider text-muted-foreground">
						{label}
					</dt>
					<dd className="text-base font-semibold tabular-nums">{value}</dd>
				</div>
			))}
		</dl>
	);
}

function Caption({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<p className="flex items-start gap-1.5 text-xs leading-5 text-muted-foreground">
			<Info className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
			{children}
		</p>
	);
}

function FamilyDot({ column }: Readonly<{ column: IShowcaseColumn }>) {
	return (
		<span
			className="h-2 w-2 shrink-0 rounded-xs"
			style={{ background: familyVar(familyOf(column)) }}
			aria-hidden="true"
		/>
	);
}

function PlaceholderCell({
	locked,
	children,
}: Readonly<{ locked?: boolean; children: ReactNode }>) {
	return (
		<span
			data-masked={locked ? "" : undefined}
			className="inline-flex select-none items-center gap-1 rounded-sm border border-dashed border-border px-1.5 py-px font-sans text-[11px] text-muted-foreground"
		>
			{locked ? <Lock className="h-3 w-3" aria-hidden="true" /> : null}
			{children}
		</span>
	);
}

const UNSAMPLED_FAMILIES = new Set(["vector", "binary", "geo"]);

const SAMPLE_FRAME =
	"overflow-hidden rounded-lg border border-border [&_tbody_tr:last-child>td]:border-b-0 [&_tr>*:last-child]:border-r-0";

function SampleGrid({ table }: Readonly<{ table: IShowcaseTable }>) {
	const { t } = useTranslation("store");
	const { columns, rows, presentation } = useMemo(() => {
		const source = table.columns ?? [];
		const placeholders = new Map<string, ReactNode>();
		for (const c of source) {
			if (c.personal)
				placeholders.set(
					c.name,
					<PlaceholderCell locked>
						{t("showcaseHidden", "hidden")}
					</PlaceholderCell>,
				);
			else if (UNSAMPLED_FAMILIES.has(c.family))
				placeholders.set(
					c.name,
					<PlaceholderCell>
						{t("showcaseNotSampled", "not in preview")}
					</PlaceholderCell>,
				);
		}
		const labels = new Map(
			source.flatMap((c) =>
				c.display_name && c.display_name !== c.name
					? [[c.name, c.display_name] as const]
					: [],
			),
		);
		return {
			columns: source.map<QueryColumn>((c, position) => ({
				name: c.name,
				type_name: c.data_type,
				position,
			})),
			rows: (table.sample?.rows ?? []).map((row) =>
				Object.fromEntries(source.map((c, i) => [c.name, row[i] ?? null])),
			),
			presentation: {
				label: (name: string) => labels.get(name) ?? name,
				cell: (name: string) => placeholders.get(name),
			} satisfies ResultTablePresentation,
		};
	}, [table.columns, table.sample, t]);

	return (
		<div className="space-y-3">
			<div className={SAMPLE_FRAME}>
				<QueryResultTable
					columns={columns}
					rows={rows}
					presentation={presentation}
					inspectRows={false}
				/>
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

function ColumnTable({ columns }: Readonly<{ columns: IShowcaseColumn[] }>) {
	const { t } = useTranslation("store");
	const labels = useFamilyLabels();
	const parts = useMemo(
		() => familyParts(familyCounts(columns), labels),
		[columns, labels],
	);
	return (
		<div className="space-y-4">
			<CompositionBar
				parts={parts}
				label={t("showcaseColumnsByType", "Columns by type")}
			/>
			<div className={gridWrapClass}>
				<table className={gridTableClass}>
					<thead>
						<tr>
							<th className={gridHeadClass}>
								{t("showcaseColumnHeader", "Column")}
							</th>
							<th className={gridHeadClass}>
								{t("showcaseTypeHeader", "Type")}
							</th>
							<th className={cn(gridHeadClass, "w-0")}>
								<span className="sr-only">
									{t("showcaseVisibilityHeader", "Visibility")}
								</span>
							</th>
						</tr>
					</thead>
					<tbody>
						{columns.map((c) => (
							<tr key={c.name} className="last:[&>td]:border-b-0">
								<td className={gridCellClass}>
									<span className="block wrap-anywhere font-mono text-[12.5px] font-medium">
										{c.name}
									</span>
									{c.description || c.display_name ? (
										<span className="mt-0.5 block text-xs text-muted-foreground">
											{c.description || c.display_name}
										</span>
									) : null}
								</td>
								<td className={gridCellClass}>
									<span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
										<span className="inline-flex items-center gap-1.5 whitespace-nowrap">
											<FamilyDot column={c} />
											{labels[familyOf(c)]}
										</span>
										<span className="wrap-anywhere font-mono text-[11px] text-muted-foreground">
											{c.vector_size
												? `${c.data_type} · ${c.vector_size}d`
												: c.data_type}
										</span>
									</span>
								</td>
								<td className={cn(gridCellClass, "whitespace-nowrap")}>
									{c.personal ? (
										<span className="inline-flex items-center gap-1 rounded-full border border-dashed border-border px-2 py-0.5 text-[11px] text-muted-foreground">
											<Lock className="h-3 w-3" aria-hidden="true" />
											{t("showcaseNeverShown", "Never shown")}
										</span>
									) : null}
								</td>
							</tr>
						))}
					</tbody>
				</table>
			</div>
		</div>
	);
}

function IndexTable({ indexes }: Readonly<{ indexes: IShowcaseIndex[] }>) {
	const { t } = useTranslation("store");
	return (
		<div className={gridWrapClass}>
			<table className={gridTableClass}>
				<thead>
					<tr>
						<th className={gridHeadClass}>
							{t("showcaseIndexHeader", "Index")}
						</th>
						<th className={gridHeadClass}>{t("showcaseTypeHeader", "Type")}</th>
						<th className={gridHeadClass}>
							{t("showcaseColumnsHeader", "Columns")}
						</th>
					</tr>
				</thead>
				<tbody>
					{indexes.map((ix) => (
						<tr key={ix.name} className="last:[&>td]:border-b-0">
							<td className={cn(gridCellClass, "font-mono text-[12.5px]")}>
								{ix.name}
							</td>
							<td className={gridCellClass}>
								<span className="rounded-sm bg-muted px-1.5 py-0.5 font-mono text-[11px]">
									{ix.index_type}
								</span>
							</td>
							<td
								className={cn(
									gridCellClass,
									"font-mono text-xs text-muted-foreground",
								)}
							>
								{ix.columns.join(", ")}
							</td>
						</tr>
					))}
				</tbody>
			</table>
		</div>
	);
}

function TableDetail({
	table,
	graph,
	onSelect,
}: Readonly<{
	table: IShowcaseTable;
	graph: ShowcaseGraph;
	onSelect: (s: ShowcaseSelection) => void;
}>) {
	const { t } = useTranslation("store");
	const facts: [string, string][] = [];
	if (typeof table.rows === "number")
		facts.push([t("showcaseStatRowsLabel", "Rows"), formatNumber(table.rows)]);
	if (table.columns)
		facts.push([
			t("showcaseStatColumnsLabel", "Columns"),
			String(table.columns.length),
		]);
	const sampleRows = table.sample?.rows.length ?? 0;
	const usedBy = graph.edges.some((e) => e.to === nodeKey("table", table.name));
	return (
		<div className="space-y-5">
			<div className="flex flex-wrap items-end justify-between gap-x-6 gap-y-3">
				{facts.length > 0 ? <Facts items={facts} /> : null}
				<div className="flex flex-wrap gap-1.5">
					<TableChips table={table} compact />
				</div>
			</div>
			<Tabs defaultValue={sampleRows > 0 ? "sample" : "columns"}>
				<TabsList className={tabsListClass}>
					<TabsTrigger value="sample" className={tabsTriggerClass}>
						{t("showcaseTabSample", "Sample rows")}
						{sampleRows > 0 ? <TabCount n={sampleRows} /> : null}
					</TabsTrigger>
					<TabsTrigger value="columns" className={tabsTriggerClass}>
						{t("showcaseTabColumns", "Columns")}
						{table.columns ? <TabCount n={table.columns.length} /> : null}
					</TabsTrigger>
					<TabsTrigger value="indexes" className={tabsTriggerClass}>
						{t("showcaseTabIndexes", "Indexes")}
						{table.indexes?.length ? (
							<TabCount n={table.indexes.length} />
						) : null}
					</TabsTrigger>
					<TabsTrigger value="used" className={tabsTriggerClass}>
						{t("showcaseTabUsedBy", "Used by")}
					</TabsTrigger>
				</TabsList>
				<TabsContent value="sample" className="pt-4">
					{sampleRows > 0 ? (
						<SampleGrid table={table} />
					) : (
						<LockedNote icon={Lock}>
							{t(
								"showcaseLockedSample",
								"The publisher doesn't share sample rows for this table. Join to explore it.",
							)}
						</LockedNote>
					)}
				</TabsContent>
				<TabsContent value="columns" className="pt-4">
					{table.columns ? (
						<ColumnTable columns={table.columns} />
					) : (
						<LockedNote icon={Lock}>
							{t(
								"showcaseLockedColumns",
								"The publisher shares this table's name only. Join to see its columns.",
							)}
						</LockedNote>
					)}
				</TabsContent>
				<TabsContent value="indexes" className="pt-4">
					{table.indexes && table.indexes.length > 0 ? (
						<IndexTable indexes={table.indexes} />
					) : (
						<LockedNote icon={Lock}>
							{t(
								"showcaseLockedIndexes",
								"Join to see how this table is indexed.",
							)}
						</LockedNote>
					)}
				</TabsContent>
				<TabsContent value="used" className="pt-4">
					{usedBy ? (
						<Related
							graph={graph}
							selection={{ kind: "table", id: table.name }}
							onSelect={onSelect}
						/>
					) : (
						<p className="text-sm text-muted-foreground">
							{t("showcaseNothingUsesIt", "Nothing else on this page uses it.")}
						</p>
					)}
				</TabsContent>
			</Tabs>
		</div>
	);
}

function EventDetail({
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
		<div className="space-y-6">
			{event.description ? (
				<p className="text-sm leading-relaxed">{event.description}</p>
			) : null}
			{event.technical ? (
				<div className="space-y-2.5">
					<SectionLabel>
						{t("showcaseTechnical", "Technical details")}
					</SectionLabel>
					{isEndpointEvent(event) ? (
						<div className="flex flex-wrap items-center gap-2.5 rounded-lg border border-border bg-muted/30 px-3.5 py-3">
							<MethodBadge method={endpointMethod(event)} />
							<span className="wrap-anywhere font-mono text-sm">
								{event.technical.path}
							</span>
						</div>
					) : (
						<div className="rounded-lg border border-border bg-muted/30 px-3.5 py-3">
							<EventTechnical event={event} />
						</div>
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
					<CodeBlock
						code={snippet}
						header={
							<>
								<Tabs
									value={lang}
									onValueChange={(v) => setLang(v as SnippetLang)}
								>
									<TabsList className="h-auto gap-4 bg-transparent p-0">
										{(
											[
												["curl", "curl"],
												["js", "JavaScript"],
												["py", "Python"],
											] as const
										).map(([value, label]) => (
											<TabsTrigger
												key={value}
												value={value}
												className={cn(
													tabsTriggerClass,
													"pt-1.5 pb-1.5 text-xs",
												)}
											>
												{label}
											</TabsTrigger>
										))}
									</TabsList>
								</Tabs>
								<CopyButton getText={getText} variant="ghost" />
							</>
						}
					/>
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

function PageDetail({
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
		<div className="space-y-6">
			{page.outline ? (
				<PageFrame route={page.route}>
					<Wireframe
						outline={page.outline}
						large
						label={t("showcasePagePreview", "Layout preview of {{name}}", {
							name: page.name,
						})}
					/>
				</PageFrame>
			) : null}
			{page.description ? (
				<p className="text-sm leading-relaxed">{page.description}</p>
			) : null}
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

function FlowDetail({
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
		<div className="space-y-6">
			{flow.description ? (
				<p className="text-sm leading-relaxed">{flow.description}</p>
			) : null}
			<Facts
				items={[
					[t("showcaseStatNodes", "Nodes"), formatNumber(flow.node_count)],
				]}
			/>
			{flow.scores ? (
				<div className="space-y-3">
					<SectionLabel>{t("showcaseQuality", "Quality")}</SectionLabel>
					<ul className="grid gap-x-8 gap-y-2.5 sm:grid-cols-2">
						{SCORE_KEYS.map((k) => {
							const v = flow.scores?.[k] ?? 0;
							return (
								<li
									key={k}
									className="grid grid-cols-[96px_1fr_28px] items-center gap-2.5 text-[13px]"
								>
									<span className="text-muted-foreground">{labels[k]}</span>
									<span
										className="h-1.5 overflow-hidden rounded-full bg-muted"
										role="img"
										aria-label={`${v}/10`}
									>
										<span
											className="block h-full rounded-full bg-primary"
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

function WidgetDetail({
	widget,
	graph,
	onSelect,
}: Readonly<{
	widget: IShowcaseWidget;
	graph: ShowcaseGraph;
	onSelect: (s: ShowcaseSelection) => void;
}>) {
	return (
		<div className="space-y-6">
			{widget.thumbnail ? (
				<img
					src={widget.thumbnail}
					alt=""
					className="aspect-video w-full rounded-lg border border-border object-cover"
				/>
			) : null}
			{widget.description ? (
				<p className="text-sm leading-relaxed">{widget.description}</p>
			) : null}
			<Related
				graph={graph}
				selection={{ kind: "widget", id: widget.id }}
				onSelect={onSelect}
			/>
		</div>
	);
}

function SourceDetail({ source }: Readonly<{ source: IShowcaseSource }>) {
	const { t } = useTranslation("store");
	return (
		<div className="space-y-5">
			<Facts items={[[t("showcaseDriver", "Driver"), source.driver]]} />
			<Caption>
				{t(
					"showcaseSourceNote",
					"Hosts and credentials stay with the publisher. Visitors only see the name and driver.",
				)}
			</Caption>
		</div>
	);
}

interface DetailModel {
	title: string;
	mono: boolean;
	kicker: string;
	badge?: ReactNode;
	wide?: boolean;
	body: ReactNode;
}

function DetailHeader({
	model,
	kind,
	icon: Icon,
}: Readonly<{
	model: DetailModel;
	kind: ShowcaseNodeKind;
	icon: typeof ShieldCheck;
}>) {
	return (
		<div className="flex items-start gap-3 border-b border-border px-5 pt-5 pb-4 pr-12">
			<span
				className={cn(
					"grid h-10 w-10 shrink-0 place-items-center rounded-lg border border-border bg-muted/60",
					NODE_TONE[kind],
				)}
			>
				<Icon className="h-5 w-5" aria-hidden="true" />
			</span>
			<div className="flex min-w-0 flex-1 flex-col gap-0.5">
				<span className="text-[11px] font-semibold uppercase tracking-widest text-muted-foreground">
					{model.kicker}
				</span>
				<div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
					<DialogTitle
						className={cn(
							"wrap-anywhere text-lg font-semibold leading-snug",
							model.mono && "font-mono text-base font-medium",
						)}
					>
						{model.title}
					</DialogTitle>
					{model.badge}
				</div>
				<DialogDescription className="sr-only">
					{model.kicker}
				</DialogDescription>
			</div>
		</div>
	);
}

/** Focus the panel itself, so opening it doesn't mark its first tab as focused. */
const focusSurface = (event: Event) => {
	event.preventDefault();
	(event.currentTarget as HTMLElement | null)?.focus();
};

/** Centered dialog on wide screens, bottom sheet on phones. */
function DetailSurface({
	open,
	wide,
	onClose,
	children,
}: Readonly<{
	open: boolean;
	wide: boolean;
	onClose: () => void;
	children: ReactNode;
}>) {
	const isMobile = useIsMobile();
	const onOpenChange = (o: boolean) => {
		if (!o) onClose();
	};
	if (isMobile) {
		return (
			<Sheet open={open} onOpenChange={onOpenChange}>
				<SheetContent
					side="bottom"
					onOpenAutoFocus={focusSurface}
					className="max-h-[92dvh] gap-0 overflow-hidden rounded-t-2xl p-0 outline-none"
				>
					<span
						aria-hidden="true"
						className="mx-auto mt-2 h-1 w-10 shrink-0 rounded-full bg-muted-foreground/30"
					/>
					{children}
				</SheetContent>
			</Sheet>
		);
	}
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent
				onOpenAutoFocus={focusSurface}
				className={cn(
					"max-h-[min(88dvh,56rem)] gap-0 overflow-hidden p-0 outline-none sm:max-w-2xl",
					wide && "sm:max-w-4xl xl:max-w-6xl",
				)}
			>
				{children}
			</DialogContent>
		</Dialog>
	);
}

export function ShowcaseDetail({
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

	const model = useMemo((): DetailModel | null => {
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
						wide: true,
						kicker: tb.object_type
							? t("showcaseKickerTable", "Table · {{title}}", {
									title: tableTitle(tb),
								})
							: t("showcaseTable", "Table"),
						body: <TableDetail table={tb} graph={graph} onSelect={onSelect} />,
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
						body: <EventDetail event={ev} graph={graph} onSelect={onSelect} />,
					}
				: null;
		}
		if (kind === "page") {
			const p = showcase.pages.find((x) => x.id === id);
			return p
				? {
						title: p.name,
						mono: false,
						wide: !!p.outline,
						kicker: t("showcasePage", "Page"),
						body: <PageDetail page={p} graph={graph} onSelect={onSelect} />,
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
						body: <FlowDetail flow={f} graph={graph} onSelect={onSelect} />,
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
						body: <WidgetDetail widget={w} graph={graph} onSelect={onSelect} />,
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
					body: <SourceDetail source={s} />,
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
		<DetailSurface open={!!model} wide={!!model?.wide} onClose={onClose}>
			{model && selection ? (
				<>
					<DetailHeader model={model} kind={selection.kind} icon={Icon} />
					<div
						key={`${selection.kind}:${selection.id}`}
						className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-5 pt-5 pb-6"
					>
						{model.body}
					</div>
				</>
			) : null}
		</DetailSurface>
	);
}

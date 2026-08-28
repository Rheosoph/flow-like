"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	BarChart3,
	Braces,
	Copy,
	Download,
	FileJson,
	Play,
	Search,
	SearchX,
	Share2,
	Sheet as SheetIcon,
	TableIcon,
	X,
} from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import type {
	ExecuteSqlResult,
	VizConfig,
	VizView,
} from "../../../../state/backend-state/query-state";
import { Badge } from "../../../ui/badge";
import { Button } from "../../../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { Input } from "../../../ui/input";
import { ScrollArea } from "../../../ui/scroll-area";
import { Skeleton } from "../../../ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../../ui/tabs";
import { cellToString, toCsv, toMarkdownTable } from "./column-types";
import { QueryResultChart, inferChartConfig } from "./query-result-chart";
import { QueryResultGraph, inferGraphConfig } from "./query-result-graph";
import { QueryResultTable } from "./query-result-table";

function download(filename: string, content: string, type: string): void {
	const blob = new Blob([content], { type });
	const url = URL.createObjectURL(blob);
	const anchor = document.createElement("a");
	anchor.href = url;
	anchor.download = filename;
	anchor.click();
	URL.revokeObjectURL(url);
}

function ResultSkeleton() {
	return (
		<div className="flex h-full flex-col" aria-hidden>
			<div className="flex h-9 items-center gap-4 border-b px-3">
				{["w-10", "w-24", "w-20", "w-28", "w-16"].map((width) => (
					<Skeleton key={width} className={`h-3 ${width}`} />
				))}
			</div>
			{Array.from({ length: 10 }, (_, index) => index).map((index) => (
				<div
					key={index}
					className="flex items-center gap-4 border-b px-3 py-2.5"
				>
					{["w-6", "w-32", "w-16", "w-40", "w-12"].map((width) => (
						<Skeleton key={width} className={`h-3 ${width} opacity-70`} />
					))}
				</div>
			))}
		</div>
	);
}

export function QueryResultView({
	result,
	loading,
	error,
	vizConfig,
	appId,
	onVizConfigChange,
	onRun,
}: Readonly<{
	result: ExecuteSqlResult | null;
	loading: boolean;
	error: string | null;
	vizConfig: VizConfig;
	/** Lets cells holding storage paths open as the files they point at. */
	appId?: string;
	onVizConfigChange: (config: VizConfig) => void;
	onRun?: () => void;
}>) {
	const { t } = useTranslation("settings");
	const view: VizView = vizConfig.view ?? "table";
	const columns = useMemo(() => result?.columns ?? [], [result]);
	const rows = useMemo(() => result?.rows ?? [], [result]);

	const [filter, setFilter] = useState("");

	// The quick-filter narrows only the on-screen table (a find affordance);
	// exports, charts, graph, and JSON always operate on the full result.
	const filteredRows = useMemo(() => {
		const needle = filter.trim().toLowerCase();
		if (!needle) return rows;
		return rows.filter((row) =>
			columns.some((column) =>
				cellToString(row[column.name]).toLowerCase().includes(needle),
			),
		);
	}, [rows, columns, filter]);

	const inferredChart = useMemo(() => inferChartConfig(columns), [columns]);
	const inferredGraph = useMemo(() => inferGraphConfig(columns), [columns]);

	const exportCsv = () =>
		download("query-result.csv", toCsv(columns, rows), "text/csv");
	const exportJson = () =>
		download(
			"query-result.json",
			JSON.stringify(rows, null, 2),
			"application/json",
		);
	const copyMarkdown = () => {
		void navigator.clipboard.writeText(toMarkdownTable(columns, rows));
		toast.success("Copied as Markdown table");
	};

	if (loading) return <ResultSkeleton />;

	if (error) {
		return (
			<div className="flex h-full flex-col items-center justify-center p-6">
				<div
					role="alert"
					className="w-full max-w-2xl overflow-hidden rounded-xl border border-destructive/30 bg-destructive/5"
				>
					<div className="flex items-center justify-between gap-2 border-b border-destructive/20 bg-destructive/10 px-4 py-2.5">
						<span className="flex items-center gap-2 text-sm font-semibold text-destructive">
							<AlertCircle className="h-4 w-4" />{" "}
							{t("queryFailed", "Query failed")}
						</span>
						<Button
							variant="ghost"
							size="sm"
							className="h-7 gap-1.5 text-destructive hover:bg-destructive/10 hover:text-destructive"
							onClick={() => {
								void navigator.clipboard.writeText(error);
								toast.success("Copied error");
							}}
						>
							<Copy className="h-3.5 w-3.5" /> {t("copy", "Copy")}
						</Button>
					</div>
					<pre className="max-h-64 overflow-auto whitespace-pre-wrap wrap-break-word px-4 py-3 font-mono text-xs text-destructive">
						{error}
					</pre>
				</div>
			</div>
		);
	}

	if (!result) {
		return (
			<div className="flex h-full flex-col items-center justify-center gap-4 p-8 text-center">
				<div className="flex items-center gap-1.5 text-muted-foreground/70">
					{[
						{ key: "table", Icon: TableIcon },
						{ key: "chart", Icon: BarChart3 },
						{ key: "graph", Icon: Share2 },
					].map(({ key, Icon }) => (
						<div key={key} className="rounded-lg border bg-muted/40 p-2">
							<Icon className="h-4 w-4" />
						</div>
					))}
				</div>
				<div className="space-y-1">
					<p className="text-sm font-medium text-foreground">
						{t("runAQueryToSeeResults", "Run a query to see results")}
					</p>
					<p className="mx-auto max-w-xs text-xs text-muted-foreground">
						{t("writeSqlAndPress", "Write SQL and press")}{" "}
						<kbd className="rounded border bg-muted px-1 font-mono text-[10px]">
							{`⌘↵`}
						</kbd>{" "}
						{t(
							"toRunResultsRenderAsATableChartOrRelationshipGraph",
							"to run. Results render as a table, chart, or relationship graph.",
						)}
					</p>
				</div>
				{onRun && (
					<Button
						variant="outline"
						size="sm"
						className="gap-1.5"
						onClick={onRun}
					>
						<Play className="h-3.5 w-3.5" /> {t("runQuery", "Run query")}
					</Button>
				)}
			</div>
		);
	}

	const matched = filteredRows.length;

	return (
		<Tabs
			value={view}
			onValueChange={(next) =>
				onVizConfigChange({ ...vizConfig, view: next as VizView })
			}
			className="flex h-full min-h-0 flex-col gap-0"
		>
			<div className="flex flex-wrap items-center justify-between gap-2 border-b bg-muted/20 p-2">
				<TabsList className="h-8">
					<TabsTrigger value="table" className="gap-1.5 text-xs">
						<TableIcon className="h-3.5 w-3.5" /> {t("table", "Table")}
					</TabsTrigger>
					<TabsTrigger value="chart" className="gap-1.5 text-xs">
						<BarChart3 className="h-3.5 w-3.5" /> {t("chart", "Chart")}
					</TabsTrigger>
					<TabsTrigger value="graph" className="gap-1.5 text-xs">
						<Share2 className="h-3.5 w-3.5" /> {t("graph", "Graph")}
					</TabsTrigger>
					<TabsTrigger value="json" className="gap-1.5 text-xs">
						<Braces className="h-3.5 w-3.5" /> {`JSON`}
					</TabsTrigger>
				</TabsList>

				<div className="flex items-center gap-2">
					{view === "table" && rows.length > 0 && (
						<div className="relative">
							<Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
							<Input
								value={filter}
								onChange={(event) => setFilter(event.target.value)}
								placeholder={t("filterRows", "Filter rows…")}
								aria-label={t("filterResultRows", "Filter result rows")}
								className="h-8 w-40 pl-8 pr-7 text-xs"
							/>
							{filter && (
								<button
									type="button"
									onClick={() => setFilter("")}
									aria-label={t("clearFilter", "Clear filter")}
									className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:text-foreground"
								>
									<X className="h-3.5 w-3.5" />
								</button>
							)}
						</div>
					)}

					<Badge variant="outline" className="gap-1 text-xs tabular-nums">
						{view === "table" && filter
							? `${matched} / ${result.row_count}`
							: result.row_count}{" "}
						row
						{result.row_count === 1 && !(view === "table" && filter) ? "" : "s"}
					</Badge>
					{result.truncated && (
						<Badge
							variant="secondary"
							className="bg-amber-500/10 text-xs text-amber-600 dark:text-amber-400"
						>
							{t("truncated", "Truncated")}
						</Badge>
					)}

					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<Button
								variant="outline"
								size="sm"
								className="h-8 gap-1.5 px-2 text-xs"
								disabled={rows.length === 0}
							>
								<Download className="h-3.5 w-3.5" /> {t("export", "Export")}
							</Button>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="end" className="w-40">
							<DropdownMenuItem onClick={exportCsv}>
								<SheetIcon className="h-3.5 w-3.5" /> CSV
							</DropdownMenuItem>
							<DropdownMenuItem onClick={exportJson}>
								<FileJson className="h-3.5 w-3.5" /> {`JSON`}
							</DropdownMenuItem>
							<DropdownMenuItem onClick={copyMarkdown}>
								<Copy className="h-3.5 w-3.5" />{" "}
								{t("copyAsMarkdown", "Copy as Markdown")}
							</DropdownMenuItem>
						</DropdownMenuContent>
					</DropdownMenu>
				</div>
			</div>

			<div className="min-h-0 flex-1">
				<TabsContent
					value="table"
					className="h-full data-[state=inactive]:hidden"
				>
					{rows.length === 0 ? (
						<EmptyResult />
					) : matched === 0 ? (
						<div className="flex h-full items-center justify-center p-8 text-center text-sm text-muted-foreground">
							<div>
								<SearchX className="mx-auto mb-2 h-6 w-6 opacity-60" />
								{t("noRowsMatchFilter", "No rows match “{{filter}}”.", {
									filter,
								})}
							</div>
						</div>
					) : (
						<QueryResultTable
							columns={columns}
							rows={filteredRows}
							appId={appId}
						/>
					)}
				</TabsContent>

				<TabsContent
					value="json"
					className="h-full data-[state=inactive]:hidden"
				>
					<ScrollArea className="h-full">
						<pre className="p-3 font-mono text-xs">
							{JSON.stringify(rows, null, 2)}
						</pre>
					</ScrollArea>
				</TabsContent>

				<TabsContent
					value="chart"
					className="h-full p-3 data-[state=inactive]:hidden"
				>
					<QueryResultChart
						columns={columns}
						rows={rows}
						config={vizConfig.chart ?? inferredChart}
						onConfigChange={(chart) =>
							onVizConfigChange({ ...vizConfig, chart })
						}
					/>
				</TabsContent>

				<TabsContent
					value="graph"
					className="h-full p-3 data-[state=inactive]:hidden"
				>
					<QueryResultGraph
						columns={columns}
						rows={rows}
						config={vizConfig.graph ?? inferredGraph}
						onConfigChange={(graph) =>
							onVizConfigChange({ ...vizConfig, graph })
						}
					/>
				</TabsContent>
			</div>
		</Tabs>
	);
}

function EmptyResult() {
	const { t } = useTranslation("settings");
	return (
		<div className="flex h-full items-center justify-center p-8 text-center">
			<div className="flex flex-col items-center gap-2 text-sm text-muted-foreground">
				<SearchX className="h-7 w-7 opacity-60" />
				<p className="font-medium text-foreground">{t("0Rows", "0 rows")}</p>
				<p>
					{t(
						"theQueryRanButMatchedNothing",
						"The query ran but matched nothing.",
					)}
				</p>
			</div>
		</div>
	);
}

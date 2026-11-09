"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	BarChart3,
	Braces,
	CheckCircle2,
	Copy,
	Database,
	Download,
	FileJson,
	FileText,
	Loader2,
	Maximize2,
	Minimize2,
	Play,
	Search,
	SearchX,
	Share2,
	Sheet as SheetIcon,
	TableIcon,
	Timer,
	X,
} from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import type {
	ExecuteSqlResult,
	VizConfig,
	VizView,
} from "../../../../state/backend-state/query-state";
import { Button } from "../../../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { Input } from "../../../ui/input";
import { Skeleton } from "../../../ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../../ui/tabs";
import { cellToString, toCsv, toMarkdownTable } from "./column-types";
import { type LastRun, formatDuration } from "./editor-footer-bar";
import { QueryResultChart, inferChartConfig } from "./query-result-chart";
import { QueryResultGraph, inferGraphConfig } from "./query-result-graph";
import { QueryResultRecord } from "./query-result-record";
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
		<div className="overflow-hidden" aria-hidden>
			{Array.from({ length: 8 }, (_, index) => index).map((index) => (
				<div key={index} className="flex items-center gap-6 border-b px-4 py-3">
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
	lastRun,
	stale = false,
	resultsExpanded = false,
	onToggleResultsExpanded,
	onVizConfigChange,
	onRun,
	onChooseTable,
	runDisabled = false,
}: Readonly<{
	result: ExecuteSqlResult | null;
	loading: boolean;
	error: string | null;
	vizConfig: VizConfig;
	/** Lets cells holding storage paths open as the files they point at. */
	appId?: string;
	lastRun?: LastRun | null;
	stale?: boolean;
	resultsExpanded?: boolean;
	onToggleResultsExpanded?: () => void;
	onVizConfigChange: (config: VizConfig) => void;
	onRun?: () => void;
	onChooseTable?: () => void;
	runDisabled?: boolean;
}>) {
	const { t } = useTranslation("settings");
	const view: VizView = vizConfig.view ?? "table";
	const columns = useMemo(() => result?.columns ?? [], [result]);
	const rows = useMemo(() => result?.rows ?? [], [result]);
	// Record is a local reading mode. Saved visualizations continue to use table.
	const [recordPreference, setRecordPreference] = useState<{
		result: ExecuteSqlResult | null;
		enabled: boolean;
	} | null>(null);
	const recordEnabled =
		recordPreference?.result === result
			? recordPreference.enabled
			: rows.length === 1;
	const displayView = view === "table" && recordEnabled ? "record" : view;
	const [filterState, setFilterState] = useState<{
		result: ExecuteSqlResult | null;
		value: string;
	}>({ result, value: "" });
	const filter = filterState.result === result ? filterState.value : "";
	const setFilter = (value: string) => setFilterState({ result, value });

	// Filtering finds rows within this table; other views and exports use the
	// full returned result. The export menu makes that scope explicit.
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
	const filtering = displayView === "table" && filter.trim().length > 0;
	const copyText = async (text: string, success: string) => {
		try {
			await navigator.clipboard.writeText(text);
			toast.success(success);
		} catch {
			toast.error(
				t("couldNotCopyToClipboard", "Could not copy to the clipboard."),
			);
		}
	};
	const expandLabel = resultsExpanded
		? t("restoreEditor", "Restore editor")
		: t("expandResults", "Expand results");

	return (
		<section
			aria-label={t("queryResults", "Query results")}
			className="@container/results flex h-full min-h-0 min-w-0 flex-col bg-background/40"
		>
			<div className="flex min-h-10 shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b px-3 py-1.5">
				<h3 className="text-xs font-semibold">{t("results", "Results")}</h3>
				<output className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
					{loading ? (
						<>
							<Loader2 className="h-3.5 w-3.5 animate-spin" />
							{t("runningQuery", "Running query…")}
						</>
					) : error ? (
						<span className="text-destructive">
							{t("queryFailed", "Query failed")}
						</span>
					) : result ? (
						<>
							<CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-600 dark:text-emerald-400" />
							<span className="tabular-nums">
								{filtering
									? t("matchingResultRows", "{{matched}} of {{total}} rows", {
											matched: filteredRows.length.toLocaleString(),
											total: rows.length.toLocaleString(),
										})
									: t(
											rows.length === 1 ? "queryResultRow" : "queryResultRows",
											rows.length === 1 ? "{{count}} row" : "{{count}} rows",
											{ count: rows.length },
										)}
							</span>
							{lastRun?.ok && (
								<span className="flex items-center gap-1 border-l pl-2 tabular-nums">
									<Timer className="h-3 w-3" />
									{formatDuration(lastRun.durationMs)}
								</span>
							)}
						</>
					) : (
						<span>{t("readyToRun", "Ready to run")}</span>
					)}
				</output>
				{onToggleResultsExpanded && (
					<Button
						variant="ghost"
						size="icon"
						className="ml-auto h-7 w-7 shrink-0 text-muted-foreground"
						onClick={onToggleResultsExpanded}
						aria-label={expandLabel}
						title={expandLabel}
						aria-pressed={resultsExpanded}
					>
						{resultsExpanded ? (
							<Minimize2 className="h-3.5 w-3.5" />
						) : (
							<Maximize2 className="h-3.5 w-3.5" />
						)}
					</Button>
				)}
			</div>

			{loading ? (
				<ResultSkeleton />
			) : error ? (
				<div className="min-h-0 flex-1 overflow-auto p-4">
					<div
						role="alert"
						className="overflow-hidden rounded-lg border border-destructive/25 bg-destructive/5"
					>
						<div className="flex items-center justify-between gap-2 border-b border-destructive/20 px-3 py-2">
							<span className="flex items-center gap-2 text-sm font-medium text-destructive">
								<AlertCircle className="h-4 w-4" />
								{t("queryCouldNotComplete", "The query could not complete")}
							</span>
							<Button
								variant="ghost"
								size="sm"
								className="h-7 gap-1.5 text-xs"
								onClick={() =>
									void copyText(error, t("copiedError", "Copied error"))
								}
							>
								<Copy className="h-3.5 w-3.5" />
								{t("copy", "Copy")}
							</Button>
						</div>
						<pre className="max-h-64 overflow-auto whitespace-pre-wrap wrap-break-word px-3 py-3 font-mono text-xs">
							{error}
						</pre>
					</div>
				</div>
			) : !result ? (
				<div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 overflow-auto p-5 text-center">
					<Database className="h-6 w-6 text-muted-foreground/50" />
					<div className="space-y-1">
						<p className="text-sm font-medium">
							{t("runAQueryToSeeResults", "Run a query to see results")}
						</p>
						<p className="max-w-sm text-xs leading-relaxed text-muted-foreground">
							{t(
								"queryResultsHint",
								"Choose a table or write SQL to explore your data.",
							)}
						</p>
					</div>
					<div className="flex flex-wrap items-center justify-center gap-2">
						{onChooseTable && (
							<Button size="sm" className="gap-1.5" onClick={onChooseTable}>
								<Database className="h-3.5 w-3.5" />
								{t("browseTables", "Browse tables")}
							</Button>
						)}
						{onRun && (
							<Button
								variant="outline"
								size="sm"
								className="gap-1.5"
								disabled={runDisabled}
								onClick={onRun}
							>
								<Play className="h-3.5 w-3.5" />
								{t("runQuery", "Run query")}
							</Button>
						)}
					</div>
				</div>
			) : (
				<Tabs
					value={displayView}
					onValueChange={(next) => {
						setRecordPreference({ result, enabled: next === "record" });
						onVizConfigChange({
							...vizConfig,
							view: next === "record" ? "table" : (next as VizView),
						});
					}}
					className="flex min-h-0 flex-1 flex-col gap-0"
				>
					{stale && (
						<output className="flex shrink-0 flex-wrap items-center gap-x-2 border-b bg-amber-500/5 px-3 py-1.5 text-xs text-amber-700 dark:text-amber-300">
							<AlertCircle className="h-3.5 w-3.5 shrink-0" />
							{t(
								"queryResultsStale",
								"Query changed. Run again to update results.",
							)}
						</output>
					)}
					{result.truncated && (
						<div className="shrink-0 border-b bg-amber-500/5 px-3 py-1.5 text-xs text-muted-foreground">
							{t(
								"queryResultsLimited",
								"Row limit reached. Increase the row limit and run again to see more. Exports include only returned rows.",
							)}
						</div>
					)}
					<div className="grid shrink-0 grid-cols-1 items-center gap-2 border-b px-3 py-2 @3xl/results:grid-cols-[auto_minmax(0,1fr)]">
						<TabsList
							className="h-8 w-full min-w-0 gap-0.5 bg-muted/50 p-0.5 @3xl/results:w-auto [&_svg]:hidden @xl/results:[&_svg]:block"
							aria-label={t("resultView", "Result view")}
						>
							<TabsTrigger
								value="record"
								disabled={rows.length === 0}
								className="gap-1.5 px-2 text-xs"
							>
								<FileText className="h-3.5 w-3.5" />
								{t("record", "Record")}
							</TabsTrigger>
							<TabsTrigger value="table" className="gap-1.5 px-2 text-xs">
								<TableIcon className="h-3.5 w-3.5" />
								{t("table", "Table")}
							</TabsTrigger>
							<TabsTrigger value="chart" className="gap-1.5 px-2 text-xs">
								<BarChart3 className="h-3.5 w-3.5" />
								{t("chart", "Chart")}
							</TabsTrigger>
							<TabsTrigger value="graph" className="gap-1.5 px-2 text-xs">
								<Share2 className="h-3.5 w-3.5" />
								{t("graph", "Graph")}
							</TabsTrigger>
							<TabsTrigger value="json" className="gap-1.5 px-2 text-xs">
								<Braces className="h-3.5 w-3.5" />
								JSON
							</TabsTrigger>
						</TabsList>
						<div className="flex min-w-0 items-center justify-end gap-2">
							{displayView === "table" && rows.length > 0 && (
								<div className="relative min-w-0 flex-1 @3xl/results:max-w-52">
									<Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
									<Input
										value={filter}
										onChange={(event) => setFilter(event.target.value)}
										placeholder={t("filterRows", "Filter rows…")}
										aria-label={t("filterResultRows", "Filter result rows")}
										className="h-8 w-full pl-8 pr-7 text-xs"
									/>
									{filter && (
										<button
											type="button"
											onClick={() => setFilter("")}
											aria-label={t("clearFilter", "Clear filter")}
											className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
										>
											<X className="h-3.5 w-3.5" />
										</button>
									)}
								</div>
							)}
							<DropdownMenu>
								<DropdownMenuTrigger asChild>
									<Button
										variant="outline"
										size="sm"
										className="h-8 shrink-0 gap-1.5 px-2 text-xs"
										disabled={rows.length === 0}
									>
										<Download className="h-3.5 w-3.5" />
										{filtering
											? t("exportAll", "Export all")
											: t("export", "Export")}
									</Button>
								</DropdownMenuTrigger>
								<DropdownMenuContent align="end" className="w-52">
									<DropdownMenuLabel className="text-xs font-normal text-muted-foreground">
										{t("exportReturnedRows", "All {{count}} returned rows", {
											count: rows.length,
										})}
									</DropdownMenuLabel>
									<DropdownMenuSeparator />
									<DropdownMenuItem
										onClick={() =>
											download(
												"query-result.csv",
												toCsv(columns, rows),
												"text/csv",
											)
										}
									>
										<SheetIcon className="h-3.5 w-3.5" />
										CSV
									</DropdownMenuItem>
									<DropdownMenuItem
										onClick={() =>
											download(
												"query-result.json",
												JSON.stringify(rows, null, 2),
												"application/json",
											)
										}
									>
										<FileJson className="h-3.5 w-3.5" />
										JSON
									</DropdownMenuItem>
									<DropdownMenuItem
										onClick={() =>
											void copyText(
												toMarkdownTable(columns, rows),
												t("copiedAsMarkdownTable", "Copied as Markdown table"),
											)
										}
									>
										<Copy className="h-3.5 w-3.5" />
										{t("copyAsMarkdown", "Copy as Markdown")}
									</DropdownMenuItem>
								</DropdownMenuContent>
							</DropdownMenu>
						</div>
					</div>
					<div className="min-h-0 flex-1">
						<TabsContent
							value="record"
							className="h-full data-[state=inactive]:hidden"
						>
							<QueryResultRecord columns={columns} rows={rows} appId={appId} />
						</TabsContent>
						<TabsContent
							value="table"
							className="h-full data-[state=inactive]:hidden"
						>
							{rows.length === 0 ? (
								<EmptyResult />
							) : filteredRows.length === 0 ? (
								<div className="flex h-full flex-col items-center justify-center gap-2 overflow-auto p-5 text-center text-xs text-muted-foreground">
									<SearchX className="h-6 w-6 opacity-60" />
									<p>
										{t("noRowsMatchFilter", "No rows match “{{filter}}”.", {
											filter,
										})}
									</p>
									<Button
										variant="outline"
										size="sm"
										onClick={() => setFilter("")}
									>
										{t("clearRowFilter", "Clear row filter")}
									</Button>
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
							className="h-full overflow-auto data-[state=inactive]:hidden"
						>
							<pre className="w-max min-w-full p-3 font-mono text-xs leading-relaxed">
								{JSON.stringify(rows, null, 2)}
							</pre>
						</TabsContent>
						<TabsContent
							value="chart"
							className="h-full overflow-auto p-3 data-[state=inactive]:hidden"
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
							className="h-full overflow-auto p-3 data-[state=inactive]:hidden"
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
			)}
		</section>
	);
}

function EmptyResult() {
	const { t } = useTranslation("settings");
	return (
		<div className="flex h-full flex-col items-center justify-center gap-2 overflow-auto p-5 text-center text-xs text-muted-foreground">
			<SearchX className="h-6 w-6 opacity-60" />
			<p className="text-sm font-medium text-foreground">
				{t("noRowsReturned", "No rows returned")}
			</p>
			<p>
				{t(
					"queryEmptyHint",
					"The query completed. Try adjusting your WHERE conditions.",
				)}
			</p>
		</div>
	);
}

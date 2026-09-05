"use client";

import {
	AlertCircle,
	Database,
	ExternalLink,
	Loader2,
	RefreshCw,
} from "lucide-react";
import Link from "next/link";
import { Suspense, lazy, useMemo } from "react";
import {
	Area,
	AreaChart,
	Bar,
	BarChart,
	CartesianGrid,
	Cell,
	Legend,
	Line,
	LineChart,
	Pie,
	PieChart,
	ResponsiveContainer,
	Scatter,
	ScatterChart,
	Tooltip,
	XAxis,
	YAxis,
} from "recharts";
import { getNivoChartTheme } from "../../lib/chart-theme";
import type {
	ExecuteSqlResult,
	QueryColumn,
} from "../../state/backend-state/query-state";
import { QueryResultTable } from "../settings/data-studio/query-workbench/query-result-table";
import { Button } from "../ui/button";
import {
	EXTENDED_HOME_DATA_VIEWS,
	HomeDataExtendedView,
} from "./data-widget-extended";
import {
	homeDataChartSeries,
	homeDataNetwork,
	homeDataNumber,
	homeDataText,
	keyHomeDataRows,
} from "./home-data-presentation";
import {
	type HomeDataConfig,
	formatHomeDataValue,
	homeDataMeasureTitle,
	normalizeHomeDataConfig,
} from "./home-data-query";
import type { IHomeWidget } from "./types";
import { useHomeData } from "./use-home-data";

const HeatMap = lazy(async () => ({
	default: (await import("@nivo/heatmap")).ResponsiveHeatMap,
}));
const Calendar = lazy(async () => ({
	default: (await import("@nivo/calendar")).ResponsiveCalendar,
}));
const TreeMap = lazy(async () => ({
	default: (await import("@nivo/treemap")).ResponsiveTreeMap,
}));
const Network = lazy(async () => ({
	default: (await import("@nivo/network")).ResponsiveNetwork,
}));
const COLORS = [
	"var(--chart-1)",
	"var(--chart-2)",
	"var(--chart-3)",
	"var(--chart-4)",
	"var(--chart-5)",
];
const tooltipStyle = {
	backgroundColor: "var(--popover)",
	color: "var(--popover-foreground)",
	border: "1px solid var(--border)",
	borderRadius: 8,
};

function RecordCard({
	row,
	columns,
	config,
	compact = false,
}: {
	row: Record<string, unknown>;
	columns: QueryColumn[];
	config: HomeDataConfig;
	compact?: boolean;
}) {
	const [first, ...rest] = columns;
	return (
		<div
			className={`min-w-0 rounded-lg border bg-background/40 ${compact ? "p-2.5" : "p-4"}`}
		>
			{first && (
				<p
					className="truncate text-sm font-semibold"
					title={homeDataText(row[first.name])}
				>
					{homeDataText(row[first.name])}
				</p>
			)}
			<dl
				className={`mt-2 grid gap-1.5 text-xs ${compact ? "" : "grid-cols-2"}`}
			>
				{rest.slice(0, compact ? 3 : 8).map((column) => (
					<div className="min-w-0" key={column.name}>
						<dt className="truncate text-muted-foreground">{column.name}</dt>
						<dd className="truncate" title={homeDataText(row[column.name])}>
							{homeDataText(row[column.name])}
						</dd>
					</div>
				))}
			</dl>
		</div>
	);
}
function HomeDataRecords({
	result,
	config,
}: { result: ExecuteSqlResult; config: HomeDataConfig }) {
	const columns =
		config.fields.length && config.mode === "records"
			? config.fields.flatMap(
					(name) => result.columns.find((column) => column.name === name) ?? [],
				)
			: result.columns;
	if (config.visualization === "table")
		return (
			<QueryResultTable
				columns={result.columns}
				rows={result.rows}
				appId={config.appId}
			/>
		);
	if (config.visualization === "record") {
		const row = result.rows[0];
		return (
			<dl className="grid gap-3 overflow-auto">
				{columns.map((column) => (
					<div key={column.name} className="border-b pb-2 last:border-0">
						<dt className="text-xs text-muted-foreground">{column.name}</dt>
						<dd className="mt-1 break-words text-sm">
							{homeDataText(row[column.name])}
						</dd>
					</div>
				))}
				{result.rows.length > 1 && (
					<p className="text-xs text-muted-foreground">
						Showing the first of {result.rows.length} returned records. Set a
						filter or sort order to choose a record.
					</p>
				)}
			</dl>
		);
	}
	if (config.visualization === "kanban") {
		if (!config.groupBy)
			return (
				<p className="text-sm text-muted-foreground">
					Choose a status column in widget settings.
				</p>
			);
		const groups = new Map<string, Record<string, unknown>[]>();
		for (const row of result.rows) {
			const label = homeDataText(row[config.groupBy]);
			if (!groups.has(label)) groups.set(label, []);
			groups.get(label)?.push(row);
		}
		return (
			<div className="flex gap-3 overflow-auto">
				{[...groups].map(([label, rows]) => (
					<section
						className="w-56 shrink-0 space-y-2 rounded-lg bg-muted/40 p-2"
						key={label}
					>
						<div className="flex justify-between gap-2 px-1 text-xs font-medium">
							<span className="truncate">{label}</span>
							<span className="text-muted-foreground">{rows.length}</span>
						</div>
						{keyHomeDataRows(rows).map(({ row, key }) => (
							<RecordCard
								key={key}
								row={row}
								columns={columns}
								config={config}
								compact
							/>
						))}
					</section>
				))}
			</div>
		);
	}
	return (
		<div
			className={
				config.visualization === "cards"
					? "grid auto-rows-min grid-cols-[repeat(auto-fit,minmax(180px,1fr))] gap-3 overflow-auto"
					: "grid auto-rows-min gap-2 overflow-auto"
			}
		>
			{keyHomeDataRows(result.rows).map(({ row, key }) => (
				<RecordCard
					key={key}
					row={row}
					columns={columns}
					config={config}
					compact={config.visualization === "list"}
				/>
			))}
		</div>
	);
}

function HomeDataChart({
	result,
	config,
}: { result: ExecuteSqlResult; config: HomeDataConfig }) {
	const { points, series } = useMemo(
		() => homeDataChartSeries(result.rows, config),
		[result.rows, config],
	);
	const nivoTheme = useMemo(() => getNivoChartTheme(), []);
	const format = (value: unknown) =>
		formatHomeDataValue(
			value,
			config.visualization === "percentstacked"
				? { ...config, format: "percent" }
				: config,
		);
	const first = series[0];
	const visualization = config.visualization;
	if (
		visualization === "percentstacked" &&
		result.rows.some((row) => {
			const share = homeDataNumber(row.__share);
			return share !== null && (share < 0 || share > 1);
		})
	)
		return (
			<p className="text-sm text-muted-foreground">
				A percentage breakdown needs nonnegative measures. Choose a different
				measure or use stacked columns.
			</p>
		);
	if (EXTENDED_HOME_DATA_VIEWS.has(visualization))
		return <HomeDataExtendedView result={result} config={config} />;
	if (visualization === "stat" || visualization === "metricstrip") {
		const row = result.rows[0];
		const measures =
			visualization === "stat" ? config.measures.slice(0, 1) : config.measures;
		return (
			<div className="flex h-full min-h-28 flex-wrap items-center gap-6">
				{measures.map((measure, index) => {
					const value = homeDataNumber(row[`__measure_${index}`]);
					return (
						<div className="min-w-28 flex-1 space-y-2" key={measure.id}>
							<p className="text-xs text-muted-foreground">
								{homeDataMeasureTitle(measure)}
							</p>
							<p
								className={`${visualization === "stat" ? "text-4xl" : "text-2xl"} font-semibold tracking-tight tabular-nums`}
							>
								{format(row[`__measure_${index}`])}
							</p>
							{config.groupBy && (
								<p className="text-xs text-muted-foreground">
									{homeDataText(row.__group)}
									{config.seriesBy ? ` · ${homeDataText(row.__series)}` : ""}
								</p>
							)}
							{config.target !== null && (
								<>
									<p className="text-xs text-muted-foreground">
										Target: {format(config.target)}
									</p>
									{config.target > 0 && value !== null && (
										<div
											className="h-1.5 overflow-hidden rounded-full bg-muted"
											role="meter"
											aria-label={`${homeDataMeasureTitle(measure)} toward target`}
											aria-valuemin={0}
											aria-valuemax={config.target}
											aria-valuenow={Math.min(
												config.target,
												Math.max(0, value),
											)}
										>
											<div
												className="h-full rounded-full bg-primary"
												style={{
													width: `${Math.min(100, Math.max(0, (value / config.target) * 100))}%`,
												}}
											/>
										</div>
									)}
								</>
							)}
						</div>
					);
				})}
				{config.groupBy && result.rows.length > 1 && (
					<p className="w-full text-xs text-muted-foreground">
						First group by the configured sort order. Remove grouping for an
						overall total.
					</p>
				)}
			</div>
		);
	}
	if (visualization === "graph") {
		if (!config.xField || !config.yField)
			return (
				<p className="text-sm text-muted-foreground">
					Choose source and target ID columns in widget settings.
				</p>
			);
		const data = homeDataNetwork(result.rows, config.xField, config.yField);
		if (!data.links.length)
			return (
				<p className="text-sm text-muted-foreground">
					No relationships have both endpoint values.
				</p>
			);
		return (
			<Network
				data={data}
				theme={nivoTheme}
				nodeColor="var(--chart-1)"
				linkColor="var(--border)"
				nodeSize={12}
				nodeBorderWidth={1}
				nodeBorderColor="var(--background)"
				linkDistance={60}
				centeringStrength={0.4}
				repulsivity={8}
				animate={false}
			/>
		);
	}
	if (visualization === "scatter") {
		if (!config.xField || !config.yField)
			return (
				<p className="text-sm text-muted-foreground">
					Choose numeric X and Y fields in widget settings.
				</p>
			);
		const data = result.rows.flatMap((row) => {
			const x = homeDataNumber(row[config.xField]);
			const y = homeDataNumber(row[config.yField]);
			return x === null || y === null ? [] : [{ x, y }];
		});
		if (!data.length)
			return (
				<p className="text-sm text-muted-foreground">
					The selected fields have no numeric pairs.
				</p>
			);
		return (
			<ResponsiveContainer
				width="100%"
				height="100%"
				minWidth={0}
				initialDimension={{ width: 1, height: 1 }}
			>
				<ScatterChart margin={{ top: 10, right: 12, bottom: 12, left: 0 }}>
					<CartesianGrid stroke="var(--border)" strokeDasharray="3 3" />
					<XAxis
						type="number"
						dataKey="x"
						name={config.xField}
						tick={{ fontSize: 11 }}
					/>
					<YAxis
						type="number"
						dataKey="y"
						name={config.yField}
						tick={{ fontSize: 11 }}
					/>
					<Tooltip contentStyle={tooltipStyle} />
					<Scatter data={data} fill={COLORS[0]} isAnimationActive={false} />
				</ScatterChart>
			</ResponsiveContainer>
		);
	}
	if (visualization === "calendar") {
		if (!config.groupBy || config.timeBucket !== "day" || config.seriesBy)
			return (
				<p className="text-sm text-muted-foreground">
					Choose a date group, daily grouping, and no series for the calendar.
				</p>
			);
		const data = result.rows.flatMap((row) => {
			const date = homeDataText(row.__group).slice(0, 10);
			const value = homeDataNumber(row.__measure_0);
			return /^\d{4}-\d{2}-\d{2}$/.test(date) && value !== null
				? [{ day: date, value }]
				: [];
		});
		if (!data.length)
			return (
				<p className="text-sm text-muted-foreground">
					No daily values are available.
				</p>
			);
		const dates = data.map((item) => item.day).sort();
		return (
			<Calendar
				data={data}
				from={dates[0]}
				to={dates[dates.length - 1]}
				theme={nivoTheme}
				emptyColor="var(--muted)"
				colors={COLORS}
				dayBorderColor="var(--background)"
				monthBorderColor="var(--border)"
				margin={{ top: 20, right: 12, bottom: 10, left: 24 }}
				yearSpacing={28}
				daySpacing={2}
			/>
		);
	}
	if (visualization === "heatmap") {
		if (!config.groupBy || !config.seriesBy)
			return (
				<p className="text-sm text-muted-foreground">
					Choose a group and a series column for the heatmap.
				</p>
			);
		const grouped = new Map<
			string,
			{ id: string; data: { x: string; y: number | null }[] }
		>();
		for (const row of result.rows) {
			const label = homeDataText(row.__series);
			if (!grouped.has(label)) grouped.set(label, { id: label, data: [] });
			grouped.get(label)?.data.push({
				x: homeDataText(row.__group),
				y: homeDataNumber(row.__measure_0),
			});
		}
		return (
			<HeatMap
				data={[...grouped.values()]}
				theme={nivoTheme}
				margin={{ top: 30, right: 12, bottom: 35, left: 80 }}
				colors={{ type: "sequential", scheme: "blues" }}
				emptyColor="var(--muted)"
				labelTextColor={{ from: "color", modifiers: [["darker", 2]] }}
				axisTop={null}
				axisBottom={{ tickRotation: -30 }}
				animate={false}
			/>
		);
	}
	if (visualization === "treemap") {
		const children = result.rows.flatMap((row, index) => {
			const value = homeDataNumber(row.__measure_0);
			return value !== null && value > 0
				? [
						{
							id: String(index),
							label: config.groupBy ? homeDataText(row.__group) : "Total",
							value,
						},
					]
				: [];
		});
		if (!children.length)
			return (
				<p className="text-sm text-muted-foreground">
					A treemap needs positive values.
				</p>
			);
		return (
			<TreeMap
				data={{ id: "root", children }}
				identity="id"
				value="value"
				label={(node) =>
					"label" in node.data ? String(node.data.label) : node.id
				}
				theme={nivoTheme}
				colors={COLORS}
				labelSkipSize={20}
				labelTextColor="var(--foreground)"
				borderWidth={2}
				borderColor="var(--background)"
				animate={false}
			/>
		);
	}
	if (!first)
		return (
			<p className="text-sm text-muted-foreground">
				Choose a measure to display.
			</p>
		);
	if (visualization === "donut" || visualization === "pie") {
		const data = result.rows.flatMap((row) => {
			const value = homeDataNumber(row.__measure_0);
			const name = `${config.groupBy ? homeDataText(row.__group) : "Total"}${config.seriesBy ? ` · ${homeDataText(row.__series)}` : ""}`;
			return value !== null && value > 0 ? [{ name, value }] : [];
		});
		if (!data.length)
			return (
				<p className="text-sm text-muted-foreground">
					A pie or donut chart needs positive values.
				</p>
			);
		return (
			<ResponsiveContainer
				width="100%"
				height="100%"
				minWidth={0}
				initialDimension={{ width: 1, height: 1 }}
			>
				<PieChart>
					<Pie
						data={data}
						dataKey="value"
						nameKey="name"
						innerRadius={visualization === "donut" ? "55%" : 0}
						outerRadius="80%"
						paddingAngle={2}
						isAnimationActive={false}
					>
						{data.map((point, index) => (
							<Cell key={point.name} fill={COLORS[index % COLORS.length]} />
						))}
					</Pie>
					<Tooltip
						formatter={(value) => format(value)}
						contentStyle={tooltipStyle}
					/>
					<Legend wrapperStyle={{ fontSize: 11 }} />
				</PieChart>
			</ResponsiveContainer>
		);
	}
	const common = {
		data: points,
		margin: { top: 10, right: 12, bottom: 8, left: 0 },
	};
	const horizontal = visualization === "horizontal";
	const axes = (
		<>
			<CartesianGrid
				stroke="var(--border)"
				strokeDasharray="3 3"
				vertical={false}
			/>
			<XAxis
				dataKey={horizontal ? undefined : "name"}
				type={horizontal ? "number" : "category"}
				tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
				tickLine={false}
				axisLine={false}
			/>
			<YAxis
				domain={visualization === "percentstacked" ? [0, 1] : undefined}
				tickFormatter={
					visualization === "percentstacked"
						? (value) => `${Math.round(Number(value) * 100)}%`
						: undefined
				}
				dataKey={horizontal ? "name" : undefined}
				type={horizontal ? "category" : "number"}
				width={horizontal ? 95 : 52}
				tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
				tickLine={false}
				axisLine={false}
			/>
			<Tooltip
				formatter={(value) => format(value)}
				contentStyle={tooltipStyle}
			/>
			{series.length > 1 && <Legend wrapperStyle={{ fontSize: 11 }} />}
		</>
	);
	if (visualization === "line")
		return (
			<ResponsiveContainer
				width="100%"
				height="100%"
				minWidth={0}
				initialDimension={{ width: 1, height: 1 }}
			>
				<LineChart {...common}>
					{axes}
					{series.map((item, index) => (
						<Line
							key={item.key}
							name={item.label}
							dataKey={item.key}
							stroke={COLORS[index % COLORS.length]}
							strokeWidth={2}
							dot={points.length < 20}
							connectNulls={false}
							isAnimationActive={false}
						/>
					))}
				</LineChart>
			</ResponsiveContainer>
		);
	if (visualization === "area")
		return (
			<ResponsiveContainer
				width="100%"
				height="100%"
				minWidth={0}
				initialDimension={{ width: 1, height: 1 }}
			>
				<AreaChart {...common}>
					{axes}
					{series.map((item, index) => (
						<Area
							key={item.key}
							name={item.label}
							dataKey={item.key}
							stroke={COLORS[index % COLORS.length]}
							fill={COLORS[index % COLORS.length]}
							fillOpacity={0.2}
							strokeWidth={2}
							connectNulls={false}
							isAnimationActive={false}
						/>
					))}
				</AreaChart>
			</ResponsiveContainer>
		);
	return (
		<ResponsiveContainer
			width="100%"
			height="100%"
			minWidth={0}
			initialDimension={{ width: 1, height: 1 }}
		>
			<BarChart {...common} layout={horizontal ? "vertical" : "horizontal"}>
				{axes}
				{series.map((item, index) => (
					<Bar
						key={item.key}
						name={item.label}
						dataKey={item.key}
						fill={COLORS[index % COLORS.length]}
						radius={3}
						stackId={
							visualization === "stacked" || visualization === "percentstacked"
								? "values"
								: undefined
						}
						isAnimationActive={false}
					/>
				))}
			</BarChart>
		</ResponsiveContainer>
	);
}

export function HomeDataWidget({
	widget,
	editing = false,
}: { widget: IHomeWidget; editing?: boolean }) {
	const config = useMemo(
		() => normalizeHomeDataConfig(widget.config),
		[widget.config],
	);
	const { result, loading, error, ready, refreshedAt, refresh } =
		useHomeData(config);
	const records = ["table", "list", "cards", "kanban", "record"].includes(
		config.visualization,
	);
	const displayResult = useMemo(() => {
		if (!result || config.mode !== "aggregate" || !records) return result;
		const names = new Map<string, string>([
			["__group", config.groupBy],
			["__series", config.seriesBy],
			...config.measures.map((measure, index): [string, string] => [
				`__measure_${index}`,
				homeDataMeasureTitle(measure),
			]),
		]);
		const used = new Set<string>();
		for (const column of result.columns) {
			const original = names.get(column.name) || column.name;
			let label = original;
			let suffix = 2;
			while (used.has(label)) label = `${original} (${suffix++})`;
			used.add(label);
			names.set(column.name, label);
		}
		return {
			...result,
			columns: result.columns.map((column) => ({
				...column,
				name: names.get(column.name) || column.name,
			})),
			rows: result.rows.map((row) =>
				Object.fromEntries(
					Object.entries(row).map(([key, value]) => [
						names.get(key) || key,
						value,
					]),
				),
			),
		};
	}, [result, config, records]);
	return (
		<div className="flex h-full min-h-0 flex-col gap-3">
			{!ready ? (
				<div className="flex min-h-36 flex-1 flex-col items-center justify-center gap-2 p-4 text-center text-muted-foreground">
					<Database className="size-7 opacity-60" />
					<p className="text-sm">Connect this widget to your data</p>
					<p className="max-w-xs text-xs">
						Choose an app, then a table, ontology object type, or saved query in
						widget settings.
					</p>
				</div>
			) : loading ? (
				<output className="flex min-h-36 flex-1 items-center justify-center gap-2 text-sm text-muted-foreground">
					<Loader2 className="size-4 animate-spin" />
					Loading data…
				</output>
			) : error ? (
				<div className="flex min-h-36 flex-1 flex-col items-start justify-center gap-2 p-2">
					<AlertCircle className="size-5 text-destructive" />
					<p className="text-sm font-medium">Data is unavailable</p>
					<p role="alert" className="break-words text-xs text-muted-foreground">
						{error}
					</p>
					<Button size="sm" variant="outline" onClick={refresh}>
						Try again
					</Button>
				</div>
			) : !result?.rows.length ? (
				<div className="flex min-h-36 flex-1 items-center justify-center p-4 text-center text-sm text-muted-foreground">
					No data matches these filters.
				</div>
			) : (
				<Suspense
					fallback={
						<div className="flex min-h-0 flex-1 items-center justify-center text-sm text-muted-foreground">
							Loading visualization…
						</div>
					}
				>
					<div
						className={`min-h-0 min-w-0 flex-1 ${records ? "flex flex-col overflow-auto" : ""}`}
					>
						{records && displayResult ? (
							<HomeDataRecords result={displayResult} config={config} />
						) : (
							<HomeDataChart result={result} config={config} />
						)}
					</div>
				</Suspense>
			)}
			{ready && (
				<div className="mt-auto flex flex-wrap items-center justify-between gap-2 border-t pt-2 text-[11px] text-muted-foreground">
					<span>
						{config.sourceKind === "query"
							? "Saved query result"
							: config.scope === "personal"
								? "Your personal data"
								: "Project data"}
						{refreshedAt
							? ` · ${refreshedAt.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`
							: ""}
						{result?.truncated ? ` · Limited to ${config.limit} results` : ""}
						{[
							"donut",
							"pie",
							"treemap",
							"heatmap",
							"calendar",
							"funnel",
							"sankey",
							"waterfall",
							"percentstacked",
						].includes(config.visualization)
							? ` · ${homeDataMeasureTitle(config.measures[0])}`
							: ""}
					</span>
					<div className="flex items-center gap-2">
						<button
							type="button"
							aria-label="Refresh widget data"
							onClick={refresh}
							disabled={loading}
							className="rounded p-1 hover:bg-muted disabled:opacity-50"
						>
							<RefreshCw className="size-3" />
						</button>
						{!editing && (
							<Link
								href={`/library/config/explore?id=${encodeURIComponent(config.appId)}`}
								aria-label="Open data source"
								className="rounded p-1 hover:bg-muted"
							>
								<ExternalLink className="size-3" />
							</Link>
						)}
					</div>
				</div>
			)}
		</div>
	);
}

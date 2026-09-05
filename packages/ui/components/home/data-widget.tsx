"use client";

import {
	AlertCircle,
	Database,
	ExternalLink,
	Loader2,
	RefreshCw,
} from "lucide-react";
import Link from "next/link";
import { Suspense, lazy, useEffect, useMemo, useRef, useState } from "react";
import {
	Area,
	AreaChart,
	Bar,
	BarChart,
	CartesianGrid,
	Cell,
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
	HomeDataCalendar,
	HomeDataLegend,
	HomeDataMessage,
} from "./data-widget-ui";
import {
	HOME_DATA_COLORS,
	homeDataAxisValue,
	homeDataCategoryLabel,
	homeDataChartSeries,
	homeDataNetwork,
	homeDataNumber,
	homeDataPresentationRequirement,
	homeDataShortLabel,
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
const TreeMap = lazy(async () => ({
	default: (await import("@nivo/treemap")).ResponsiveTreeMap,
}));
const Network = lazy(async () => ({
	default: (await import("@nivo/network")).ResponsiveNetwork,
}));
const COLORS = HOME_DATA_COLORS;
const tooltipStyle = {
	backgroundColor: "var(--popover)",
	color: "var(--popover-foreground)",
	border: "1px solid var(--border)",
	borderRadius: 8,
	fontSize: 12,
	boxShadow: "0 8px 24px rgb(0 0 0 / 0.12)",
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
			className={`min-w-0 rounded-lg ${compact ? "border-b border-border/60 px-1 py-3 last:border-b-0" : "border border-border/60 bg-muted/15 p-3"}`}
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
				className={`mt-1.5 grid gap-x-3 gap-y-1.5 text-xs ${compact ? "grid-cols-2" : "grid-cols-[repeat(auto-fit,minmax(min(100%,100px),1fr))]"}`}
			>
				{rest.slice(0, compact ? 3 : 8).map((column) => (
					<div className="min-w-0" key={column.name}>
						<dt className="truncate text-muted-foreground">{column.name}</dt>
						<dd className="truncate" title={homeDataText(row[column.name])}>
							{typeof row[column.name] === "number"
								? formatHomeDataValue(row[column.name], config)
								: homeDataText(row[column.name])}
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
			<dl className="grid content-start gap-3 overflow-auto">
				{columns.map((column) => (
					<div
						key={column.name}
						className="grid grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)] gap-3 border-b border-border/50 pb-2 last:border-0"
					>
						<dt className="text-xs text-muted-foreground">{column.name}</dt>
						<dd className="break-words text-sm">
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
				<HomeDataMessage title="Nothing to display">
					Choose a status column in widget settings.
				</HomeDataMessage>
			);
		const groups = new Map<string, Record<string, unknown>[]>();
		for (const row of result.rows) {
			const label = homeDataText(row[config.groupBy]);
			if (!groups.has(label)) groups.set(label, []);
			groups.get(label)?.push(row);
		}
		return (
			<div className="flex h-full min-h-0 items-start gap-3 overflow-auto">
				{[...groups].map(([label, rows]) => (
					<section
						className="max-h-full w-52 shrink-0 space-y-1 overflow-auto rounded-lg bg-muted/30 p-2"
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
					? "grid auto-rows-min grid-cols-[repeat(auto-fit,minmax(min(100%,180px),1fr))] gap-2 overflow-auto"
					: "grid auto-rows-min overflow-auto"
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
	width = 360,
}: { result: ExecuteSqlResult; config: HomeDataConfig; width?: number }) {
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
			<HomeDataMessage title="Nothing to display">
				A percentage breakdown needs nonnegative measures. Choose a different
				measure or use stacked columns.
			</HomeDataMessage>
		);
	if (EXTENDED_HOME_DATA_VIEWS.has(visualization))
		return (
			<HomeDataExtendedView result={result} config={config} width={width} />
		);
	if (visualization === "stat" || visualization === "metricstrip") {
		const row = result.rows[0];
		const measures =
			visualization === "stat" ? config.measures.slice(0, 1) : config.measures;
		return (
			<div
				className="grid h-full min-h-0 content-start items-start gap-x-3 gap-y-3 overflow-auto"
				style={{
					gridTemplateColumns: `repeat(${Math.min(measures.length, Math.max(1, Math.floor(width / 90)))}, minmax(0, 1fr))`,
				}}
			>
				{measures.map((measure, index) => {
					const value = homeDataNumber(row[`__measure_${index}`]);
					return (
						<div className="min-w-0 space-y-1.5" key={measure.id}>
							<p className="text-xs text-muted-foreground">
								{homeDataMeasureTitle(measure)}
							</p>
							<p
								className={`${visualization === "stat" ? "text-[clamp(1.65rem,4cqw,2.4rem)]" : "text-[clamp(1.3rem,3.4cqw,1.8rem)]"} break-words font-semibold leading-tight tracking-tight tabular-nums`}
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
				<HomeDataMessage title="Nothing to display">
					Choose source and target ID columns in widget settings.
				</HomeDataMessage>
			);
		const data = homeDataNetwork(result.rows, config.xField, config.yField);
		if (!data.links.length)
			return (
				<HomeDataMessage title="Nothing to display">
					No relationships have both endpoint values.
				</HomeDataMessage>
			);
		return (
			<div className="flex h-full min-h-0 flex-col">
				<div className="min-h-0 flex-1">
					<Network
						data={data}
						theme={nivoTheme}
						nodeColor="var(--chart-1)"
						linkColor="var(--muted-foreground)"
						nodeSize={10}
						nodeBorderWidth={1}
						nodeBorderColor="var(--background)"
						linkDistance={60}
						centeringStrength={0.4}
						repulsivity={8}
						animate={false}
					/>
				</div>
				<p className="pt-1 text-[10px] text-muted-foreground">
					{data.nodes.length} objects · {data.links.length} relationships shown
				</p>
			</div>
		);
	}
	if (visualization === "scatter") {
		if (!config.xField || !config.yField)
			return (
				<HomeDataMessage title="Nothing to display">
					Choose numeric X and Y fields in widget settings.
				</HomeDataMessage>
			);
		const data = result.rows.flatMap((row) => {
			const x = homeDataNumber(row[config.xField]);
			const y = homeDataNumber(row[config.yField]);
			return x === null || y === null ? [] : [{ x, y }];
		});
		if (!data.length)
			return (
				<HomeDataMessage title="Nothing to display">
					The selected fields have no numeric pairs.
				</HomeDataMessage>
			);
		return (
			<ResponsiveContainer
				width="100%"
				height="100%"
				minWidth={0}
				initialDimension={{ width: 1, height: 1 }}
			>
				<ScatterChart margin={{ top: 8, right: 8, bottom: 0, left: 0 }}>
					<CartesianGrid
						stroke="var(--border)"
						strokeOpacity={0.6}
						strokeDasharray="2 4"
					/>
					<XAxis
						type="number"
						dataKey="x"
						name={config.xField}
						tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
						tickLine={false}
						axisLine={false}
					/>
					<YAxis
						type="number"
						dataKey="y"
						name={config.yField}
						tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
						tickLine={false}
						axisLine={false}
						width={45}
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
				<HomeDataMessage title="Nothing to display">
					Choose a date group, daily grouping, and no series for the calendar.
				</HomeDataMessage>
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
				<HomeDataMessage title="Nothing to display">
					No daily values are available.
				</HomeDataMessage>
			);
		return <HomeDataCalendar data={data} format={format} />;
	}
	if (visualization === "heatmap") {
		if (!config.groupBy || !config.seriesBy)
			return (
				<HomeDataMessage title="Nothing to display">
					Choose a group and a series column for the heatmap.
				</HomeDataMessage>
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
				margin={{
					top: 8,
					right: 8,
					bottom: 30,
					left: Math.min(80, width * 0.24),
				}}
				colors={{ type: "sequential", scheme: "blues" }}
				emptyColor="var(--muted)"
				labelTextColor={{ from: "color", modifiers: [["darker", 2]] }}
				axisTop={null}
				axisBottom={{
					tickSize: 0,
					tickPadding: 10,
					format: (value) => homeDataCategoryLabel(value, 12),
				}}
				axisLeft={{
					tickSize: 0,
					tickPadding: 8,
					format: (value) => homeDataShortLabel(value, width < 360 ? 12 : 18),
				}}
				label={(cell) => homeDataAxisValue(cell.value, config)}
				valueFormat={(value) => format(value)}
				enableLabels={result.rows.length < 80}
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
							label: `${config.groupBy ? homeDataText(row.__group) : "Total"}${config.seriesBy ? ` · ${homeDataText(row.__series)}` : ""}`,
							value,
						},
					]
				: [];
		});
		if (!children.length)
			return (
				<HomeDataMessage title="Nothing to display">
					A treemap needs positive values.
				</HomeDataMessage>
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
				valueFormat={(value) => format(value)}
				tooltip={(node) => (
					<div className="rounded-lg border bg-popover p-2 text-xs text-popover-foreground shadow-lg">
						{"label" in node.node.data
							? String(node.node.data.label)
							: node.node.id}
						: {format(node.node.value)}
					</div>
				)}
				labelTextColor="var(--foreground)"
				borderWidth={2}
				borderColor="var(--background)"
				animate={false}
			/>
		);
	}
	if (!first)
		return (
			<HomeDataMessage title="Nothing to display">
				Choose a measure to display.
			</HomeDataMessage>
		);
	if (visualization === "donut" || visualization === "pie") {
		const data = result.rows.flatMap((row) => {
			const value = homeDataNumber(row.__measure_0);
			const name = `${config.groupBy ? homeDataText(row.__group) : "Total"}${config.seriesBy ? ` · ${homeDataText(row.__series)}` : ""}`;
			return value !== null && value > 0 ? [{ name, value }] : [];
		});
		if (!data.length)
			return (
				<HomeDataMessage title="Nothing to display">
					A pie or donut chart needs positive values.
				</HomeDataMessage>
			);
		return (
			<div className="flex h-full min-h-0 flex-col">
				<div className="min-h-0 flex-1">
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
								outerRadius="84%"
								stroke="var(--card)"
								strokeWidth={3}
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
						</PieChart>
					</ResponsiveContainer>
				</div>
				<HomeDataLegend
					items={data.map((item) => ({
						key: item.name,
						label: item.name,
						value: format(item.value),
					}))}
				/>
			</div>
		);
	}
	if (
		!points.some((point) =>
			series.some((item) => homeDataNumber(point[item.key]) !== null),
		)
	)
		return (
			<HomeDataMessage title="No numeric values to plot">
				Choose a numeric measure or adjust the filters.
			</HomeDataMessage>
		);
	const horizontal = visualization === "horizontal";
	const numericFormat = (value: unknown) =>
		homeDataAxisValue(
			value,
			visualization === "percentstacked"
				? { ...config, format: "percent" }
				: config,
		);
	const common = {
		data: points,
		margin: { top: 8, right: 8, bottom: 0, left: 0 },
	};
	const axes = (
		<>
			<CartesianGrid
				stroke="var(--border)"
				strokeOpacity={0.65}
				strokeDasharray="2 4"
				vertical={horizontal}
				horizontal={!horizontal}
			/>
			<XAxis
				dataKey={horizontal ? undefined : "name"}
				type={horizontal ? "number" : "category"}
				tickFormatter={
					horizontal
						? numericFormat
						: (value) => homeDataCategoryLabel(value, width < 360 ? 12 : 18)
				}
				tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
				tickLine={false}
				axisLine={false}
				minTickGap={18}
				height={26}
			/>
			<YAxis
				domain={visualization === "percentstacked" ? [0, 1] : undefined}
				ticks={
					visualization === "percentstacked"
						? [0, 0.25, 0.5, 0.75, 1]
						: undefined
				}
				allowDataOverflow={visualization === "percentstacked"}
				tickFormatter={
					horizontal
						? (value) => homeDataShortLabel(value, width < 360 ? 13 : 18)
						: numericFormat
				}
				dataKey={horizontal ? "name" : undefined}
				type={horizontal ? "category" : "number"}
				width={horizontal ? Math.min(110, width * 0.32) : 50}
				tickCount={4}
				tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
				tickLine={false}
				axisLine={false}
			/>
			<Tooltip
				formatter={(value) => format(value)}
				contentStyle={tooltipStyle}
				cursor={{ fill: "var(--muted)", fillOpacity: 0.35 }}
			/>
		</>
	);
	return (
		<div className="flex h-full min-h-0 flex-col">
			<div className="min-h-0 flex-1 overflow-auto">
				<div
					className="h-full min-w-0"
					style={
						horizontal
							? { minHeight: Math.min(points.length * 29 + 30, 15000) }
							: undefined
					}
				>
					<ResponsiveContainer
						width="100%"
						height="100%"
						minWidth={0}
						initialDimension={{ width: 1, height: 1 }}
					>
						{visualization === "line" ? (
							<LineChart {...common}>
								{axes}
								{series.map((item, index) => (
									<Line
										key={item.key}
										name={item.label}
										dataKey={item.key}
										stroke={COLORS[index % COLORS.length]}
										strokeWidth={2.25}
										dot={
											points.length <= 12 ? { r: 2.5, strokeWidth: 1.5 } : false
										}
										activeDot={{ r: 4 }}
										connectNulls={false}
										isAnimationActive={false}
									/>
								))}
							</LineChart>
						) : visualization === "area" ? (
							<AreaChart {...common}>
								{axes}
								{series.map((item, index) => (
									<Area
										key={item.key}
										name={item.label}
										dataKey={item.key}
										stroke={COLORS[index % COLORS.length]}
										fill={COLORS[index % COLORS.length]}
										fillOpacity={0.14}
										strokeWidth={2}
										dot={points.length === 1 ? { r: 3 } : false}
										connectNulls={false}
										isAnimationActive={false}
									/>
								))}
							</AreaChart>
						) : (
							<BarChart
								{...common}
								layout={horizontal ? "vertical" : "horizontal"}
								barCategoryGap={visualization === "histogram" ? 1 : "24%"}
							>
								{axes}
								{series.map((item, index) => (
									<Bar
										key={item.key}
										name={item.label}
										dataKey={item.key}
										fill={COLORS[index % COLORS.length]}
										radius={horizontal ? [0, 3, 3, 0] : [3, 3, 0, 0]}
										maxBarSize={horizontal ? 20 : width < 320 ? 30 : 42}
										stackId={
											visualization === "stacked" ||
											visualization === "percentstacked"
												? "values"
												: undefined
										}
										isAnimationActive={false}
									/>
								))}
							</BarChart>
						)}
					</ResponsiveContainer>
				</div>
			</div>
			{series.length > 1 && <HomeDataLegend items={series} />}
		</div>
	);
}

export function HomeDataWidget({
	widget,
	editing = false,
}: { widget: IHomeWidget; editing?: boolean }) {
	const container = useRef<HTMLDivElement>(null);
	const [width, setWidth] = useState(360);
	useEffect(() => {
		const element = container.current;
		if (!element) return;
		const observer = new ResizeObserver(([entry]) =>
			setWidth(entry.contentRect.width),
		);
		observer.observe(element);
		return () => observer.disconnect();
	}, []);
	const config = useMemo(
		() => normalizeHomeDataConfig(widget.config),
		[widget.config],
	);
	const requirement = homeDataPresentationRequirement(config);
	const { result, loading, error, ready, refreshedAt, refresh } = useHomeData(
		config,
		!requirement,
	);
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
	const state =
		!ready || requirement
			? "unconfigured"
			: loading
				? "loading"
				: error
					? "error"
					: !result?.rows.length
						? "empty"
						: "ready";
	const filtered = config.filters.length > 0 || config.dateRange !== "all";
	const summary = [
		"stat",
		"metricstrip",
		"progress",
		"gauge",
		"bullet",
	].includes(config.visualization);
	const source =
		config.sourceKind === "query"
			? "Saved query"
			: config.sourceKind === "ontology"
				? config.objectType
				: config.table;
	const updated = refreshedAt?.toLocaleTimeString([], {
		hour: "2-digit",
		minute: "2-digit",
	});
	return (
		<div
			ref={container}
			className={`flex min-h-0 min-w-0 flex-col gap-2 ${state === "ready" ? "h-full" : ""}`}
			style={{ containerType: "inline-size" }}
			data-home-data-state={state}
			data-home-data-presentation={config.visualization}
		>
			{state === "unconfigured" ? (
				<HomeDataMessage
					title={!ready ? "Connect your data" : "Choose the chart fields"}
					icon={Database}
				>
					{!ready
						? "Choose an app and a table, ontology, or saved query in widget settings."
						: requirement}
				</HomeDataMessage>
			) : state === "loading" ? (
				<output className="flex items-center gap-2.5 py-5 text-xs text-muted-foreground">
					<Loader2 className="size-4 animate-spin" aria-hidden="true" />
					Loading data…
				</output>
			) : state === "error" ? (
				<HomeDataMessage
					title="Data is unavailable"
					icon={AlertCircle}
					action={
						<Button
							type="button"
							size="sm"
							variant="outline"
							className="h-7 text-xs"
							onClick={refresh}
						>
							<RefreshCw className="mr-1.5 size-3" />
							Try again
						</Button>
					}
				>
					<p>Check the source and try again.</p>
					<details className="mt-1">
						<summary className="cursor-pointer text-[11px]">
							Error details
						</summary>
						<p role="alert" className="mt-1 max-h-24 overflow-auto break-all">
							{error}
						</p>
					</details>
				</HomeDataMessage>
			) : state === "empty" ? (
				<HomeDataMessage
					title={filtered ? "No data matches these filters." : "No records yet"}
				>
					{filtered
						? "Try a broader time range or adjust the filters."
						: "This source has no records to display."}
				</HomeDataMessage>
			) : (
				result && (
					<Suspense
						fallback={
							<output className="py-5 text-xs text-muted-foreground">
								Loading visualization…
							</output>
						}
					>
						{!summary &&
							!records &&
							!["recordcalendar", "timeline", "comparison", "graph"].includes(
								config.visualization,
							) && (
								<div className="flex shrink-0 items-center justify-between gap-2 text-[11px] text-muted-foreground">
									<span className="truncate">
										{config.visualization === "scatter"
											? `${config.xField} · ${config.yField}`
											: config.visualization === "boxplot"
												? config.yField
												: homeDataMeasureTitle(config.measures[0])}
									</span>
									{config.groupBy && (
										<span className="truncate text-right">
											by {config.groupBy}
										</span>
									)}
								</div>
							)}
						<div
							className={`min-h-0 min-w-0 flex-1 ${records ? "flex flex-col overflow-auto" : ""}`}
						>
							{records && displayResult ? (
								<HomeDataRecords result={displayResult} config={config} />
							) : (
								<HomeDataChart result={result} config={config} width={width} />
							)}
						</div>
					</Suspense>
				)
			)}
			{ready && (
				<div className="flex shrink-0 items-center justify-between gap-2 border-t border-border/50 pt-2 text-[10px] leading-4 text-muted-foreground">
					<div className="min-w-0 flex-1">
						<div
							className="flex min-w-0 items-center gap-1.5"
							title={`${source} · ${config.scope === "personal" ? "Personal data" : "Project data"}${updated ? ` · Updated ${updated}` : ""}`}
						>
							<Database
								className="size-3 shrink-0 opacity-60"
								aria-hidden="true"
							/>
							<span className="truncate">{source}</span>
							{width >= 340 && updated && (
								<span className="shrink-0 opacity-70">· {updated}</span>
							)}
						</div>
						{result?.truncated && (
							<p className="mt-0.5 text-amber-600 dark:text-amber-400">
								Limited to {config.limit} results
							</p>
						)}
					</div>
					<div className="flex shrink-0 items-center gap-0.5">
						<button
							type="button"
							aria-label="Refresh widget data"
							title={
								updated ? `Refresh · Updated ${updated}` : "Refresh widget data"
							}
							onClick={refresh}
							disabled={loading}
							className="rounded-md p-1.5 transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
						>
							<RefreshCw className="size-3" />
						</button>
						{!editing && (
							<Link
								href={`/library/config/explore?id=${encodeURIComponent(config.appId)}`}
								aria-label="Open data source"
								title="Open data source"
								className="rounded-md p-1.5 transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
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

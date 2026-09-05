"use client";

import { lazy, useMemo } from "react";
import {
	Bar,
	BarChart,
	Cell,
	ReferenceLine,
	ResponsiveContainer,
	Tooltip,
	XAxis,
	YAxis,
} from "recharts";
import { getNivoChartTheme } from "../../lib/chart-theme";
import type { ExecuteSqlResult } from "../../state/backend-state/query-state";
import { HomeDataLegend, HomeDataMessage } from "./data-widget-ui";
import {
	HOME_DATA_COLORS,
	homeDataAxisValue,
	homeDataCategoryLabel,
	homeDataChartSeries,
	homeDataNumber,
	homeDataShortLabel,
	homeDataText,
	keyHomeDataRows,
} from "./home-data-presentation";
import {
	type HomeDataConfig,
	formatHomeDataValue,
	homeDataMeasureTitle,
} from "./home-data-query";

const Sankey = lazy(async () => ({
	default: (await import("@nivo/sankey")).ResponsiveSankey,
}));
const Funnel = lazy(async () => ({
	default: (await import("@nivo/funnel")).ResponsiveFunnel,
}));
const COLORS = HOME_DATA_COLORS;
export const EXTENDED_HOME_DATA_VIEWS = new Set([
	"progress",
	"gauge",
	"bullet",
	"boxplot",
	"funnel",
	"waterfall",
	"sankey",
	"pivot",
	"timeline",
	"recordcalendar",
	"comparison",
]);

export function HomeDataExtendedView({
	result,
	config,
	width = 360,
}: { result: ExecuteSqlResult; config: HomeDataConfig; width?: number }) {
	const theme = useMemo(() => getNivoChartTheme(), []);
	const format = (value: unknown) => formatHomeDataValue(value, config);
	const view = config.visualization;
	const firstRow = result.rows[0];
	if (["progress", "gauge", "bullet"].includes(view)) {
		const value = homeDataNumber(firstRow?.__measure_0);
		if (value === null)
			return (
				<HomeDataMessage title="Nothing to display">
					This measure has no numeric value.
				</HomeDataMessage>
			);
		if (config.target === null || config.target <= 0)
			return (
				<HomeDataMessage title="Nothing to display">
					Set a target greater than zero in widget settings.
				</HomeDataMessage>
			);
		const fraction = Math.max(0, Math.min(1, value / config.target));
		const maximum = Math.max(config.target, value, 0) * 1.15;
		return (
			<div className="flex h-full min-h-0 flex-col justify-center gap-3 overflow-auto">
				{view === "gauge" ? (
					<svg
						viewBox="0 0 220 175"
						className="mx-auto min-h-0 w-full max-w-52 flex-1"
						role="img"
						aria-label={`${format(value)} of target ${format(config.target)}`}
					>
						<title>
							{format(value)} of target {format(config.target)}
						</title>
						<circle
							cx={110}
							cy={83}
							r={60}
							fill="none"
							stroke="var(--muted)"
							strokeWidth={12}
						/>
						<circle
							cx={110}
							cy={83}
							r={60}
							fill="none"
							stroke="var(--chart-1)"
							strokeWidth={12}
							strokeLinecap="round"
							strokeDasharray={`${fraction * Math.PI * 120} ${Math.PI * 120}`}
							transform="rotate(-90 110 83)"
						/>
						<text
							x={110}
							y={84}
							fill="currentColor"
							textAnchor="middle"
							fontSize={22}
							fontWeight={600}
						>
							{homeDataAxisValue(value, config)}
						</text>
						<text
							x={110}
							y={104}
							fill="var(--muted-foreground)"
							textAnchor="middle"
							fontSize={10}
						>
							Target {format(config.target)}
						</text>
					</svg>
				) : (
					<>
						<div className="flex items-baseline justify-between gap-2">
							<span className="min-w-0 break-words text-[clamp(1.65rem,4cqw,2.25rem)] font-semibold leading-tight tabular-nums">
								{format(value)}
							</span>
							<span className="text-xs text-muted-foreground">
								Target {format(config.target)}
							</span>
						</div>
						{view === "progress" ? (
							<div
								role="meter"
								className="h-2 w-full shrink-0 overflow-hidden rounded-full bg-muted"
								aria-valuemin={0}
								aria-valuemax={config.target}
								aria-valuenow={Math.min(config.target, Math.max(0, value))}
								aria-label={homeDataMeasureTitle(config.measures[0])}
							>
								<div
									className="h-full rounded-full bg-primary"
									style={{ width: `${fraction * 100}%` }}
								/>
							</div>
						) : (
							<div className="relative h-6 shrink-0 rounded bg-muted/60">
								<div
									className="absolute inset-y-2 left-0 rounded bg-primary"
									style={{ width: `${Math.max(0, (value / maximum) * 100)}%` }}
								/>
								<div
									className="absolute inset-y-0 w-0.5 bg-foreground"
									style={{ left: `${(config.target / maximum) * 100}%` }}
									title={`Target ${format(config.target)}`}
								/>
							</div>
						)}
					</>
				)}
				<p className="text-xs text-muted-foreground">
					{homeDataMeasureTitle(config.measures[0])} ·{" "}
					{new Intl.NumberFormat(undefined, {
						style: "percent",
						maximumFractionDigits: 1,
					}).format(value / config.target)}{" "}
					of target
					{config.groupBy
						? ` · ${homeDataText(firstRow.__group)} (first group)`
						: ""}
				</p>
			</div>
		);
	}
	if (view === "boxplot") {
		const data = result.rows.flatMap((row) => {
			const values = [
				row.__min,
				row.__q1,
				row.__measure_0,
				row.__q3,
				row.__max,
			].map(homeDataNumber);
			return values.some((value) => value === null)
				? []
				: [
						{
							label: `${config.groupBy ? homeDataText(row.__group) : "All records"}${config.seriesBy ? ` · ${homeDataText(row.__series)}` : ""}`,
							values: values as number[],
						},
					];
		});
		if (!data.length)
			return (
				<HomeDataMessage title="Nothing to display">
					Choose a numeric distribution field in widget settings.
				</HomeDataMessage>
			);
		const min = Math.min(...data.map((item) => item.values[0]));
		const max = Math.max(...data.map((item) => item.values[4]));
		const span = max - min || 1;
		const y = (value: number) => 190 - ((value - min) / span) * 160;
		const plotWidth = Math.max(width, 60 + data.length * 56);
		return (
			<div className="flex h-full min-h-0 flex-col gap-1">
				<div className="min-h-0 flex-1 overflow-auto">
					<svg
						viewBox={`0 0 ${plotWidth} 240`}
						className="h-full min-h-0 w-full"
						style={{ minWidth: plotWidth }}
						preserveAspectRatio="none"
						role="img"
						aria-label={`Distribution of ${config.yField}`}
					>
						<title>
							Distribution of {config.yField}. Quartiles are approximate;
							whiskers show minimum and maximum.
						</title>
						{[0, 0.25, 0.5, 0.75, 1].map((fraction) => (
							<g key={fraction}>
								<line
									x1={55}
									x2={plotWidth}
									y1={y(min + span * fraction)}
									y2={y(min + span * fraction)}
									stroke="var(--border)"
								/>
								<text
									x={50}
									y={y(min + span * fraction) + 3}
									textAnchor="end"
									fontSize={9}
									fill="var(--muted-foreground)"
								>
									{format(min + span * fraction)}
								</text>
							</g>
						))}
						{data.map((item, index) => {
							const x = 55 + ((index + 0.5) / data.length) * (plotWidth - 65);
							return (
								<g key={item.label}>
									<title>
										{item.label}: min {format(item.values[0])}, Q1{" "}
										{format(item.values[1])}, median {format(item.values[2])},
										Q3 {format(item.values[3])}, max {format(item.values[4])}
									</title>
									<line
										x1={x}
										x2={x}
										y1={y(item.values[0])}
										y2={y(item.values[4])}
										stroke="var(--chart-1)"
									/>
									<line
										x1={x - 10}
										x2={x + 10}
										y1={y(item.values[0])}
										y2={y(item.values[0])}
										stroke="var(--chart-1)"
									/>
									<line
										x1={x - 10}
										x2={x + 10}
										y1={y(item.values[4])}
										y2={y(item.values[4])}
										stroke="var(--chart-1)"
									/>
									<rect
										x={x - 20}
										y={y(item.values[3])}
										width={40}
										height={Math.max(1, y(item.values[1]) - y(item.values[3]))}
										fill="var(--chart-1)"
										opacity={0.6}
									/>
									<line
										x1={x - 20}
										x2={x + 20}
										y1={y(item.values[2])}
										y2={y(item.values[2])}
										stroke="var(--foreground)"
										strokeWidth={2}
									/>
									<text
										x={x}
										y={215}
										textAnchor="middle"
										fontSize={10}
										fill="var(--muted-foreground)"
									>
										{homeDataShortLabel(item.label, 11)}
									</text>
								</g>
							);
						})}
					</svg>
				</div>
				<p className="shrink-0 text-[10px] leading-4 text-muted-foreground">
					Approximate quartiles; median, minimum and maximum are computed on the
					source.
				</p>
			</div>
		);
	}
	if (view === "pivot") {
		if (!config.groupBy || !config.seriesBy)
			return (
				<HomeDataMessage title="Nothing to display">
					Choose a row group and a column series for the pivot table.
				</HomeDataMessage>
			);
		const pivot = homeDataChartSeries(result.rows, config);
		return (
			<div className="h-full overflow-auto">
				<table className="w-full border-collapse text-left text-xs">
					<thead className="sticky top-0 z-10 bg-card">
						<tr className="border-b">
							<th className="sticky left-0 bg-background p-2">
								{config.groupBy}
							</th>
							{pivot.series.map((series) => (
								<th
									className="whitespace-nowrap p-2 font-medium"
									key={series.key}
								>
									{series.label}
								</th>
							))}
						</tr>
					</thead>
					<tbody>
						{pivot.points.map((row) => (
							<tr
								key={homeDataText(row.name)}
								className="border-b border-border/50 last:border-0 hover:bg-muted/20"
							>
								<th className="sticky left-0 bg-background p-2 font-medium">
									{homeDataText(row.name)}
								</th>
								{pivot.series.map((series) => (
									<td
										className="whitespace-nowrap p-2 tabular-nums"
										key={series.key}
									>
										{format(row[series.key])}
									</td>
								))}
							</tr>
						))}
					</tbody>
				</table>
			</div>
		);
	}
	if (view === "sankey") {
		if (!config.groupBy || !config.seriesBy)
			return (
				<HomeDataMessage title="Nothing to display">
					Choose source groups and destination series for the Sankey.
				</HomeDataMessage>
			);
		const labels = new Map<string, string>();
		const links = result.rows.flatMap((row) => {
			const value = homeDataNumber(row.__measure_0);
			if (value === null || value <= 0) return [];
			const from = homeDataText(row.__group);
			const to = homeDataText(row.__series);
			const source = `from:${from}`;
			const target = `to:${to}`;
			labels.set(source, from);
			labels.set(target, to);
			return [{ source, target, value }];
		});
		if (!links.length)
			return (
				<HomeDataMessage title="Nothing to display">
					A Sankey needs positive values between groups.
				</HomeDataMessage>
			);
		return (
			<Sankey
				data={{
					nodes: [...labels].map(([id, label]) => ({ id, label })),
					links,
				}}
				label={(node) =>
					homeDataShortLabel(
						labels.get(node.id) ?? node.id,
						width < 360 ? 15 : 22,
					)
				}
				theme={theme}
				colors={COLORS}
				margin={{ top: 8, right: 8, bottom: 8, left: 8 }}
				nodeThickness={10}
				nodeSpacing={10}
				nodeBorderWidth={0}
				linkBlendMode="normal"
				linkOpacity={0.32}
				nodeTooltip={({ node }) => (
					<div className="rounded-lg border bg-popover p-2 text-xs text-popover-foreground shadow-lg">
						{labels.get(node.id) ?? node.id}: {format(node.value)}
					</div>
				)}
				linkTooltip={({ link }) => (
					<div className="rounded-lg border bg-popover p-2 text-xs text-popover-foreground shadow-lg">
						{labels.get(link.source.id)} → {labels.get(link.target.id)}:{" "}
						{format(link.value)}
					</div>
				)}
				labelTextColor="var(--foreground)"
				animate={false}
			/>
		);
	}
	if (view === "funnel" || view === "waterfall") {
		const order = new Map(
			config.categoryOrder.map((value, index) => [value, index]),
		);
		const data = result.rows.flatMap((row) => {
			const value = homeDataNumber(row.__measure_0);
			return value === null
				? []
				: [
						{
							id: `${homeDataText(row.__group)}${config.seriesBy ? ` · ${homeDataText(row.__series)}` : ""}`,
							label: config.groupBy ? homeDataText(row.__group) : "Total",
							value,
						},
					];
		});
		if (order.size)
			data.sort(
				(a, b) =>
					(order.get(a.label) ?? Number.MAX_SAFE_INTEGER) -
					(order.get(b.label) ?? Number.MAX_SAFE_INTEGER),
			);
		if (view === "funnel") {
			if (
				!data.length ||
				!data.some((item) => item.value > 0) ||
				data.some((item) => item.value < 0)
			)
				return (
					<HomeDataMessage title="Nothing to display">
						A funnel needs nonnegative stage values.
					</HomeDataMessage>
				);
			return (
				<div className="flex h-full min-h-0 flex-col">
					<div className="min-h-0 flex-1">
						<Funnel
							data={data}
							theme={theme}
							colors={COLORS}
							direction={width < 440 ? "vertical" : "horizontal"}
							margin={{ top: 12, right: 15, bottom: 12, left: 15 }}
							valueFormat={(value) => format(value)}
							labelColor="var(--foreground)"
							borderWidth={0}
							animate={false}
						/>
					</div>
					<HomeDataLegend
						items={data.map((item) => ({
							key: item.id,
							label: item.label,
							value: format(item.value),
						}))}
					/>
				</div>
			);
		}
		let total = config.baseline;
		const changes = data.map((item) => {
			const start = total;
			total += item.value;
			return {
				name: item.label,
				range: [Math.min(start, total), Math.max(start, total)],
				change: item.value,
			};
		});
		return (
			<div className="flex h-full flex-col">
				<div className="min-h-0 flex-1">
					<ResponsiveContainer
						width="100%"
						height="100%"
						minWidth={0}
						initialDimension={{ width: 1, height: 1 }}
					>
						<BarChart
							data={changes}
							margin={{ top: 10, left: 0, right: 10, bottom: 10 }}
						>
							<XAxis
								dataKey="name"
								tickFormatter={(value) => homeDataCategoryLabel(value, 12)}
								tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
								axisLine={false}
								tickLine={false}
								minTickGap={16}
							/>
							<YAxis
								tickFormatter={(value) => homeDataAxisValue(value, config)}
								tick={{ fontSize: 10, fill: "var(--muted-foreground)" }}
								axisLine={false}
								tickLine={false}
								width={48}
								tickCount={4}
							/>
							<ReferenceLine y={0} stroke="var(--border)" />
							<Tooltip
								formatter={(_value, _name, item) => format(item.payload.change)}
								contentStyle={{
									background: "var(--popover)",
									border: "1px solid var(--border)",
									borderRadius: 8,
								}}
							/>
							<Bar
								dataKey="range"
								name="Change"
								maxBarSize={36}
								radius={[3, 3, 0, 0]}
								isAnimationActive={false}
							>
								{changes.map((item) => (
									<Cell
										key={item.name}
										fill={
											item.change >= 0 ? "var(--chart-1)" : "var(--destructive)"
										}
									/>
								))}
							</Bar>
						</BarChart>
					</ResponsiveContainer>
				</div>
				<p className="text-[11px] text-muted-foreground">
					From {format(config.baseline)} to {format(total)} across the displayed
					groups.
				</p>
			</div>
		);
	}
	const columns = config.fields.length
		? config.fields
		: result.columns.map((column) => column.name);
	if (view === "comparison") {
		const rows = keyHomeDataRows(result.rows.slice(0, 10));
		return (
			<div className="h-full overflow-auto">
				<table className="w-full border-collapse text-left text-xs">
					<thead className="sticky top-0 z-10 bg-card">
						<tr className="border-b">
							<th className="p-2">Property</th>
							{rows.map(({ key, row }) => (
								<th key={key} className="min-w-32 p-2">
									{homeDataText(row[columns[0]])}
								</th>
							))}
						</tr>
					</thead>
					<tbody>
						{columns.slice(1).map((field) => (
							<tr
								key={field}
								className="border-b border-border/50 last:border-0 hover:bg-muted/20"
							>
								<th className="p-2 font-medium text-muted-foreground">
									{field}
								</th>
								{rows.map(({ key, row }) => (
									<td key={key} className="max-w-60 break-words p-2">
										{homeDataText(row[field])}
									</td>
								))}
							</tr>
						))}
					</tbody>
				</table>
				{result.rows.length > 10 && (
					<p className="text-xs text-muted-foreground">
						Comparing the first 10 returned records. Filter the source to select
						a smaller set.
					</p>
				)}
			</div>
		);
	}
	if (view === "timeline" || view === "recordcalendar") {
		if (!config.xField)
			return (
				<HomeDataMessage title="Nothing to display">
					Choose a date column in widget settings.
				</HomeDataMessage>
			);
		const dated = keyHomeDataRows(result.rows)
			.flatMap((item) => {
				const value = item.row[config.xField];
				if (value === null || value === undefined) return [];
				const date = new Date(
					typeof value === "number" ? value : homeDataText(value),
				);
				return Number.isNaN(date.getTime())
					? []
					: [{ ...item, date, day: date.toISOString().slice(0, 10) }];
			})
			.sort((a, b) => a.date.getTime() - b.date.getTime());
		if (!dated.length)
			return (
				<HomeDataMessage title="Nothing to display">
					No records have a valid date in this field.
				</HomeDataMessage>
			);
		if (view === "timeline")
			return (
				<ol className="h-full space-y-3 overflow-auto pl-2">
					{dated.map(({ key, row, date }) => (
						<li
							key={key}
							className="relative border-l border-primary/25 pb-2 pl-4"
						>
							<span className="absolute -left-[5px] top-1 size-2 rounded-full bg-primary" />
							<time
								className="text-[11px] text-muted-foreground"
								dateTime={date.toISOString()}
							>
								{/^\d{4}-\d{2}-\d{2}$/.test(homeDataText(row[config.xField]))
									? date.toLocaleDateString(undefined, {
											month: "short",
											day: "numeric",
											year: "numeric",
											timeZone: "UTC",
										})
									: date.toLocaleString(undefined, {
											month: "short",
											day: "numeric",
											hour: "2-digit",
											minute: "2-digit",
										})}
							</time>
							<p className="truncate text-sm font-medium">
								{homeDataText(row[columns[0]])}
							</p>
							{columns
								.slice(1, 4)
								.filter((field) => field !== config.xField)
								.map((field) => (
									<p className="text-xs text-muted-foreground" key={field}>
										{field}: {homeDataText(row[field])}
									</p>
								))}
						</li>
					))}
				</ol>
			);
		const months = [
			...new Set(dated.map((item) => item.day.slice(0, 7))),
		].slice(0, 24);
		return (
			<div className="h-full space-y-5 overflow-auto">
				{months.map((month) => {
					const [year, monthNumber] = month.split("-").map(Number);
					const start = new Date(Date.UTC(year, monthNumber - 1, 1));
					const offset = (start.getUTCDay() + 6) % 7;
					const days = new Date(Date.UTC(year, monthNumber, 0)).getUTCDate();
					return (
						<section key={month}>
							<p className="mb-2 text-sm font-medium">
								{start.toLocaleDateString(undefined, {
									month: "long",
									year: "numeric",
									timeZone: "UTC",
								})}
							</p>
							<div className="grid grid-cols-7 text-[10px]">
								{["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map(
									(label) => (
										<span className="p-1 text-muted-foreground" key={label}>
											{label}
										</span>
									),
								)}
								{Array.from({ length: days }, (_, index) => {
									const day = index + 1;
									const date = `${month}-${String(day).padStart(2, "0")}`;
									const items = dated.filter((item) => item.day === date);
									return (
										<div
											key={date}
											style={
												day === 1 ? { gridColumnStart: offset + 1 } : undefined
											}
											className="min-h-14 min-w-0 border border-border/50 p-1"
										>
											<span className="text-muted-foreground">{day}</span>
											{items.map(({ key, row }) => (
												<p
													key={key}
													className="mt-1 truncate rounded bg-primary/15 px-1 py-0.5"
													title={homeDataText(row[columns[0]])}
												>
													{homeDataText(row[columns[0]])}
												</p>
											))}
										</div>
									);
								})}
							</div>
						</section>
					);
				})}
				<p className="text-[11px] text-muted-foreground">
					Dates shown in UTC. Up to 24 months from the returned records.
				</p>
			</div>
		);
	}
	return null;
}

"use client";

import { useTranslation } from "@flow-like/locales";
import { Settings2 } from "lucide-react";
import { useMemo } from "react";
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
	Scatter,
	ScatterChart,
	XAxis,
	YAxis,
} from "recharts";
import type {
	ChartType,
	QueryColumn,
	VizChartConfig,
} from "../../../../state/backend-state/query-state";
import { Button } from "../../../ui/button";
import {
	type ChartConfig,
	ChartContainer,
	ChartLegend,
	ChartLegendContent,
	ChartTooltip,
	ChartTooltipContent,
} from "../../../ui/chart";
import { Label } from "../../../ui/label";
import { Popover, PopoverContent, PopoverTrigger } from "../../../ui/popover";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { humanizeIdentifier } from "../data-studio-panels";
import { isNumericColumn } from "./column-types";

const CHART_COLORS = [
	"var(--chart-1)",
	"var(--chart-2)",
	"var(--chart-3)",
	"var(--chart-4)",
	"var(--chart-5)",
];

const CHART_TYPES: { value: ChartType; label: string }[] = [
	{ value: "bar", label: "Bar" },
	{ value: "line", label: "Line" },
	{ value: "area", label: "Area" },
	{ value: "pie", label: "Pie" },
	{ value: "scatter", label: "Scatter" },
];

export function inferChartConfig(columns: QueryColumn[]): VizChartConfig {
	const numeric = columns.filter(isNumericColumn);
	const category = columns.find((column) => !isNumericColumn(column));
	return {
		type: "bar",
		x: category?.name ?? columns[0]?.name,
		y: numeric.slice(0, 3).map((column) => column.name),
	};
}

function color(index: number): string {
	return CHART_COLORS[index % CHART_COLORS.length];
}

export function QueryResultChart({
	columns,
	rows,
	config,
	onConfigChange,
}: Readonly<{
	columns: QueryColumn[];
	rows: Record<string, unknown>[];
	config: VizChartConfig;
	onConfigChange: (config: VizChartConfig) => void;
}>) {
	const { t } = useTranslation("settings");
	const numericColumns = useMemo(
		() => columns.filter(isNumericColumn),
		[columns],
	);

	const yKeys = config.y ?? [];
	const xKey = config.x ?? columns[0]?.name ?? "";

	const chartData = useMemo(() => {
		return rows.map((row) => {
			const point: Record<string, unknown> = { ...row };
			for (const key of yKeys) {
				const value = row[key];
				point[key] =
					value === null || value === undefined ? null : Number(value);
			}
			return point;
		});
	}, [rows, yKeys]);

	const chartConfig = useMemo<ChartConfig>(() => {
		const entries: ChartConfig = {};
		yKeys.forEach((key, index) => {
			entries[key] = {
				label: humanizeIdentifier(key),
				color: color(index),
			};
		});
		return entries;
	}, [yKeys]);

	const update = (partial: Partial<VizChartConfig>) =>
		onConfigChange({ ...config, ...partial });

	const single = config.type === "pie" || config.type === "scatter";
	const canRender =
		rows.length > 0 && xKey && (single ? yKeys[0] : yKeys.length > 0);

	const summary = t('typeChartOfValByVal2', '{{type}} chart of {{val}} by {{val2}}', { type: config.type, val: yKeys
		.map(humanizeIdentifier)
		.join(", "), val2: humanizeIdentifier(xKey) });

	return (
		<div className="relative flex h-full min-h-0 flex-col">
			<div className="absolute right-0 top-0 z-10">
				<Popover>
					<PopoverTrigger asChild>
						<Button variant="outline" size="sm" className="h-8 gap-1.5">
							<Settings2 className="h-3.5 w-3.5" /> {t('configure', 'Configure')}
						</Button>
					</PopoverTrigger>
					<PopoverContent align="end" className="w-72 space-y-3">
						<div className="grid gap-1.5">
							<Label className="text-xs text-muted-foreground">
								{t('chartType', 'Chart type')}
							</Label>
							<Select
								value={config.type}
								onValueChange={(value) => update({ type: value as ChartType })}
							>
								<SelectTrigger className="h-8" aria-label={t('chartType', 'Chart type')}>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{CHART_TYPES.map((item) => (
										<SelectItem key={item.value} value={item.value}>
											{item.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="grid gap-1.5">
							<Label className="text-xs text-muted-foreground">
								{config.type === "pie" ? "Label" : t('xCategory', 'X / Category')}
							</Label>
							<Select
								value={xKey}
								onValueChange={(value) => update({ x: value })}
							>
								<SelectTrigger
									className="h-8"
									aria-label={t('xOrCategoryColumn', 'X or category column')}
								>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{columns.map((column) => (
										<SelectItem key={column.name} value={column.name}>
											{humanizeIdentifier(column.name)}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="grid gap-1.5">
							<Label className="text-xs text-muted-foreground">
								{single ? "Value" : "Values"}
							</Label>
							<div className="flex flex-wrap gap-1.5">
								{numericColumns.length === 0 && (
									<span className="text-xs text-muted-foreground">
										{t('noNumericColumns', 'No numeric columns')}
									</span>
								)}
								{numericColumns.map((column) => {
									const active = yKeys.includes(column.name);
									return (
										<button
											key={column.name}
											type="button"
											aria-pressed={active}
											onClick={() =>
												update({
													y: single
														? [column.name]
														: active
															? yKeys.filter((key) => key !== column.name)
															: [...yKeys, column.name],
												})
											}
											className={`rounded-full border px-2.5 py-1 text-xs transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring ${
												active
													? "border-primary bg-primary/10 text-primary"
													: "hover:bg-muted"
											}`}
										>
											{humanizeIdentifier(column.name)}
										</button>
									);
								})}
							</div>
						</div>
					</PopoverContent>
				</Popover>
			</div>

			{!canRender ? (
				<div className="flex h-full items-center justify-center rounded-lg border border-dashed text-sm text-muted-foreground">
					{t('pickACategoryAndAtLeastOneValueColumnToChart', 'Pick a category and at least one value column to chart.')}
				</div>
			) : (
				<ChartContainer
					config={chartConfig}
					className="aspect-auto h-full w-full"
					role="img"
					aria-label={summary}
				>
					{config.type === "bar" ? (
						<BarChart data={chartData}>
							<CartesianGrid vertical={false} />
							<XAxis dataKey={xKey} tickLine={false} axisLine={false} />
							<YAxis tickLine={false} axisLine={false} />
							<ChartTooltip content={<ChartTooltipContent />} />
							<ChartLegend
								content={({ payload, verticalAlign }) => (
									<ChartLegendContent
										payload={payload}
										verticalAlign={verticalAlign}
									/>
								)}
							/>
							{yKeys.map((key, index) => (
								<Bar key={key} dataKey={key} fill={color(index)} radius={4} />
							))}
						</BarChart>
					) : config.type === "line" ? (
						<LineChart data={chartData}>
							<CartesianGrid vertical={false} />
							<XAxis dataKey={xKey} tickLine={false} axisLine={false} />
							<YAxis tickLine={false} axisLine={false} />
							<ChartTooltip content={<ChartTooltipContent />} />
							<ChartLegend
								content={({ payload, verticalAlign }) => (
									<ChartLegendContent
										payload={payload}
										verticalAlign={verticalAlign}
									/>
								)}
							/>
							{yKeys.map((key, index) => (
								<Line
									key={key}
									type="monotone"
									dataKey={key}
									stroke={color(index)}
									strokeWidth={2}
									dot={false}
								/>
							))}
						</LineChart>
					) : config.type === "area" ? (
						<AreaChart data={chartData}>
							<CartesianGrid vertical={false} />
							<XAxis dataKey={xKey} tickLine={false} axisLine={false} />
							<YAxis tickLine={false} axisLine={false} />
							<ChartTooltip content={<ChartTooltipContent />} />
							<ChartLegend
								content={({ payload, verticalAlign }) => (
									<ChartLegendContent
										payload={payload}
										verticalAlign={verticalAlign}
									/>
								)}
							/>
							{yKeys.map((key, index) => (
								<Area
									key={key}
									type="monotone"
									dataKey={key}
									stroke={color(index)}
									fill={color(index)}
									fillOpacity={0.2}
								/>
							))}
						</AreaChart>
					) : config.type === "pie" ? (
						<PieChart>
							<ChartTooltip content={<ChartTooltipContent hideLabel />} />
							<Pie
								data={chartData}
								dataKey={yKeys[0]}
								nameKey={xKey}
								outerRadius="80%"
							>
								{chartData.map((entry, index) => (
									<Cell
										key={`${String(entry[xKey])}-${index}`}
										fill={color(index)}
									/>
								))}
							</Pie>
						</PieChart>
					) : (
						<ScatterChart>
							<CartesianGrid />
							<XAxis
								dataKey={xKey}
								name={xKey}
								type="category"
								tickLine={false}
								axisLine={false}
							/>
							<YAxis
								dataKey={yKeys[0]}
								name={yKeys[0]}
								type="number"
								tickLine={false}
								axisLine={false}
							/>
							<ChartTooltip
								cursor={{ strokeDasharray: "3 3" }}
								content={<ChartTooltipContent />}
							/>
							<Scatter data={chartData} fill={color(0)} />
						</ScatterChart>
					)}
				</ChartContainer>
			)}
		</div>
	);
}

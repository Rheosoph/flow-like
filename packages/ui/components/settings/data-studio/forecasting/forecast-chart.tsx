"use client";

import * as SliderPrimitive from "@radix-ui/react-slider";
import {
	Activity,
	ArrowRight,
	Clock3,
	Loader2,
	Maximize2,
	Minimize2,
	RotateCcw,
	ScanLine,
	ZoomIn,
	ZoomOut,
} from "lucide-react";
import { useEffect, useId, useState } from "react";
import {
	Area,
	CartesianGrid,
	ComposedChart,
	Line,
	LineChart,
	ReferenceArea,
	ReferenceLine,
	ResponsiveContainer,
	Tooltip,
	XAxis,
	YAxis,
} from "recharts";
import { cn } from "../../../../lib/utils";
import { Button } from "../../../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogTitle,
	DialogTrigger,
} from "../../../ui/dialog";
import { Switch } from "../../../ui/switch";
import {
	type ForecastPlotPoint,
	forecastBoundarySegment,
	formatForecastNumber,
	formatForecastTick,
	formatForecastTime,
} from "./forecast-data";

const FORECAST_COLOR = "var(--chart-1)";

export interface CovariateTimeline {
	name: string;
	role: "past" | "known";
	enabled: boolean;
	disabled?: boolean;
	points: { timestamp: number; value: number | null }[];
}

export function ForecastChart({
	points,
	cutoff,
	metric,
	seriesLabel,
	seriesColumn,
	seriesCount,
	hasEvaluationMetrics = false,
	quantileLevels,
	pointStatistic = "median",
	loading,
	stale,
	failedRefresh = false,
	backtest,
	covariates = [],
	onCovariateToggle,
	canUseCovariates = true,
	readOnly = false,
	compact = false,
	height,
	showControls = true,
}: Readonly<{
	points: ForecastPlotPoint[];
	cutoff?: number;
	metric: string;
	seriesLabel?: string;
	seriesColumn?: string;
	seriesCount?: number;
	hasEvaluationMetrics?: boolean;
	quantileLevels: number[];
	pointStatistic?: "mean" | "median";
	loading: boolean;
	stale: boolean;
	failedRefresh?: boolean;
	backtest: boolean;
	covariates?: CovariateTimeline[];
	onCovariateToggle?: (name: string, enabled: boolean) => void;
	canUseCovariates?: boolean;
	readOnly?: boolean;
	compact?: boolean;
	height?: number;
	showControls?: boolean;
}>) {
	const chartId = `forecast-${useId().replaceAll(":", "")}`;
	const [timelineGutter, setTimelineGutter] = useState(compact ? 64 : 88);
	const [expanded, setExpanded] = useState(false);
	useEffect(() => {
		if (!showControls) setExpanded(false);
	}, [showControls]);
	const identity = JSON.stringify([
		metric,
		seriesLabel,
		seriesColumn,
		backtest,
		cutoff,
	]);
	const [selection, setSelection] = useState<{
		points: ForecastPlotPoint[];
		identity: string;
		range: [number, number];
	} | null>(null);
	const [drag, setDrag] = useState<[number, number] | null>(null);
	const lastIndex = Math.max(0, points.length - 1);
	const range: [number, number] =
		selection?.points === points && selection.identity === identity
			? selection.range
			: [0, lastIndex];
	const canZoom = points.length > 2;
	const zoomed = range[0] > 0 || range[1] < lastIndex;
	const visiblePoints = points.slice(range[0], range[1] + 1);
	const setRange = (range: [number, number]) => {
		setSelection({ points, identity, range });
		setDrag(null);
	};
	const resetZoom = () => setRange([0, lastIndex]);
	const zoom = (factor: number) => {
		const span = Math.max(
			1,
			Math.min(lastIndex, Math.round((range[1] - range[0]) * factor)),
		);
		const start = Math.max(
			0,
			Math.min(lastIndex - span, Math.round((range[0] + range[1] - span) / 2)),
		);
		setRange([start, start + span]);
	};
	const focusForecast = () => {
		const start = points.findIndex((point) => point.forecast !== null);
		if (start < 0) return;
		setRange([
			Math.max(0, start - Math.max(1, Math.round((lastIndex - start) * 0.1))),
			lastIndex,
		]);
	};
	const finishDrag = () => {
		if (!drag || drag[0] === drag[1]) {
			setDrag(null);
			return;
		}
		const start = points.findIndex(
			(point) => point.timestamp >= Math.min(...drag),
		);
		const end = points.findLastIndex(
			(point) => point.timestamp <= Math.max(...drag),
		);
		if (start >= 0 && end > start) setRange([start, end]);
		else setDrag(null);
	};
	const plotHeight =
		typeof height === "number" && Number.isFinite(height)
			? Math.max(120, Math.min(1200, height))
			: compact
				? 220
				: undefined;
	const levels = [...quantileLevels].sort((a, b) => a - b);
	const bandLabel =
		levels.length > 1
			? `P${Math.round(levels[0] * 100)}–P${Math.round(levels[levels.length - 1] * 100)}`
			: "Quantiles";
	const lastObserved = points.findLast((point) => point.history !== null);
	const latestForecast = points.findLast((point) => point.forecast !== null);
	const transition = forecastBoundarySegment(points, cutoff);
	const visibleTransition =
		transition &&
		visiblePoints.length > 0 &&
		visiblePoints[0].timestamp <= transition[0].x &&
		visiblePoints[visiblePoints.length - 1].timestamp >= transition[1].x
			? transition
			: undefined;
	const groupCount =
		typeof seriesCount === "number" && Number.isFinite(seriesCount)
			? Math.max(1, Math.floor(seriesCount))
			: 1;
	const grouped = Boolean(seriesColumn) || groupCount > 1;
	const selectedSeries = seriesLabel
		? `${seriesColumn || (grouped ? "Group" : "Series")}: ${seriesLabel}`
		: undefined;
	const domain: [number, number] | undefined = visiblePoints.length
		? [
				visiblePoints[0].timestamp,
				visiblePoints[visiblePoints.length - 1].timestamp,
			]
		: undefined;
	const renderChart = (fullscreen: boolean) => {
		const gradientId = `${chartId}-${fullscreen ? "expanded" : "inline"}`;
		const cutoffVisible =
			cutoff !== undefined &&
			domain &&
			cutoff >= domain[0] &&
			cutoff <= domain[1];
		return (
			<section
				className={cn(
					"min-w-0 overflow-hidden rounded-xl border bg-card text-card-foreground shadow-sm",
					fullscreen && "min-h-full shrink-0 rounded-none border-0 shadow-none",
				)}
				aria-label="Forecast visualization"
				aria-busy={loading}
			>
				<div
					className={cn(
						"flex flex-wrap items-start justify-between gap-4 pb-2",
						compact ? "px-3 pt-3" : "px-4 pt-4 sm:px-5 sm:pt-5",
					)}
				>
					<div>
						<p className="flex items-center gap-2 text-xs font-medium uppercase tracking-[0.18em] text-muted-foreground">
							<Activity className="size-3.5 text-chart-1" />
							{backtest ? "Holdout evaluation" : "Forecast outlook"}
						</p>
						<h3
							className={cn(
								"mt-2 break-words font-medium tracking-tight",
								compact ? "text-base" : "text-xl",
							)}
						>
							{metric && (
								<span className="mr-2 text-sm font-normal text-muted-foreground">
									Target
								</span>
							)}
							{metric || "Your next forecast"}
						</h3>
						{points.length > 0 && (
							<div className="mt-2 space-y-1.5" aria-label="Forecast scope">
								<div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 text-xs">
									<span className="rounded-full border border-border px-2 py-0.5 text-foreground">
										{grouped ? "Per-group forecasts" : "Single series"}
									</span>
									{selectedSeries && (
										<span className="break-words text-foreground">
											{selectedSeries}
										</span>
									)}
									{grouped && (
										<span className="text-muted-foreground">
											{groupCount > 1
												? `1 of ${groupCount.toLocaleString()} groups shown`
												: "1 group"}
										</span>
									)}
								</div>
								{grouped && (
									<p className="max-w-lg text-xs leading-relaxed text-muted-foreground">
										Each group has its own forecast. An overall forecast needs
										an aggregated source.
									</p>
								)}
							</div>
						)}
						<div className="mt-3 flex flex-wrap items-baseline gap-x-5 gap-y-1">
							{lastObserved && (
								<span
									className={cn(
										"font-medium tabular-nums",
										compact ? "text-lg" : "text-2xl",
									)}
								>
									{formatForecastNumber(lastObserved.history)}
									<span className="ml-2 text-xs font-normal text-muted-foreground">
										last observed
									</span>
								</span>
							)}
							{latestForecast && (
								<span className="flex items-center gap-1.5 text-sm tabular-nums text-chart-1">
									<ArrowRight className="size-3.5" />
									{formatForecastNumber(latestForecast.forecast)}
									<span className="text-xs text-muted-foreground">
										final {pointStatistic}
									</span>
								</span>
							)}
						</div>
					</div>
					<div className="flex flex-col items-end gap-3 text-xs">
						{loading ? (
							<span className="flex items-center gap-2 text-chart-1">
								<Loader2 className="size-3.5 animate-spin" />
								Running model
							</span>
						) : stale ? (
							<span className="rounded-full border border-border bg-muted px-2.5 py-1 text-muted-foreground">
								{failedRefresh ? "Previous result" : "Settings changed"}
							</span>
						) : cutoff !== undefined ? (
							<span className="flex items-center gap-1.5 text-muted-foreground">
								<Clock3 className="size-3.5" />
								{formatForecastTime(cutoff, true)} UTC
							</span>
						) : null}
						<div className="flex flex-wrap justify-end gap-4 text-xs text-muted-foreground">
							<span className="flex items-center gap-1.5">
								<i className="h-0.5 w-4 bg-foreground" />
								History
							</span>
							<span className="flex items-center gap-1.5">
								<i className="h-0.5 w-4 bg-chart-1" />
								{pointStatistic === "mean" ? "Mean" : "Median"}
							</span>
							{levels.length > 1 && (
								<span className="flex items-center gap-1.5">
									<i className="h-2 w-4 rounded-sm bg-chart-1/25" />
									{bandLabel}
								</span>
							)}
							{levels.length === 0 && latestForecast && (
								<span className="text-muted-foreground">
									Point forecast only
								</span>
							)}
							{backtest && (
								<span className="flex items-center gap-1.5">
									<i className="w-4 border-t border-dashed border-muted-foreground" />
									Held-out actual
								</span>
							)}
						</div>
					</div>
				</div>
				{showControls && (points.length > 0 || fullscreen) && (
					<div
						className={cn(
							"flex flex-wrap items-center justify-between gap-2 border-y bg-muted/20 py-2",
							compact ? "px-3" : "px-4 sm:px-5",
						)}
						aria-label="Chart controls"
					>
						<div className="flex flex-wrap items-center gap-1">
							<Button
								type="button"
								variant="ghost"
								size="icon"
								className="size-8"
								aria-label="Zoom in"
								title="Zoom in"
								disabled={!canZoom || range[1] - range[0] <= 1}
								onClick={() => zoom(0.5)}
							>
								<ZoomIn className="size-4" />
							</Button>
							<Button
								type="button"
								variant="ghost"
								size="icon"
								className="size-8"
								aria-label="Zoom out"
								title="Zoom out"
								disabled={!zoomed}
								onClick={() => zoom(2)}
							>
								<ZoomOut className="size-4" />
							</Button>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								className="h-8 gap-1.5 px-2 text-xs"
								disabled={!zoomed}
								onClick={resetZoom}
							>
								<RotateCcw className="size-3.5" />
								Reset zoom
							</Button>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								className="h-8 gap-1.5 px-2 text-xs"
								disabled={!canZoom || !latestForecast}
								onClick={focusForecast}
							>
								<ScanLine className="size-3.5" />
								{backtest ? "Focus holdout" : "Focus forecast"}
							</Button>
						</div>
						{fullscreen ? (
							<Button
								type="button"
								variant="outline"
								size="sm"
								className="h-8 gap-1.5 text-xs"
								onClick={() => setExpanded(false)}
							>
								<Minimize2 className="size-3.5" />
								Exit full screen
							</Button>
						) : (
							<DialogTrigger asChild>
								<Button
									type="button"
									variant="outline"
									size="sm"
									className="h-8 gap-1.5 text-xs"
								>
									<Maximize2 className="size-3.5" />
									Full screen
								</Button>
							</DialogTrigger>
						)}
					</div>
				)}
				<div
					className={cn(
						"relative w-full pb-3 pt-3",
						showControls &&
							canZoom &&
							"select-none [&_.recharts-wrapper]:cursor-crosshair",
						fullscreen && "h-[max(280px,55dvh)]",
						compact ? "px-3" : "px-4 sm:px-5",
						!fullscreen && plotHeight === undefined && "h-[320px] sm:h-[380px]",
					)}
					style={
						fullscreen || plotHeight === undefined
							? undefined
							: { height: plotHeight }
					}
				>
					{points.length > 0 ? (
						<ResponsiveContainer
							width="100%"
							height="100%"
							minWidth={0}
							initialDimension={{ width: 600, height: 300 }}
							onResize={(width) =>
								setTimelineGutter(compact || width < 480 ? 64 : 88)
							}
						>
							<ComposedChart
								data={visiblePoints}
								margin={{ top: 12, right: 48, left: 0, bottom: 0 }}
								accessibilityLayer
								onMouseDown={(state, event) => {
									if (
										!showControls ||
										!canZoom ||
										("button" in event && event.button !== 0)
									)
										return;
									const time = Number(state.activeLabel);
									if (state.activeLabel != null && Number.isFinite(time))
										setDrag([time, time]);
								}}
								onMouseMove={(state) => {
									const time = Number(state.activeLabel);
									if (
										drag &&
										state.activeLabel != null &&
										Number.isFinite(time)
									)
										setDrag([drag[0], time]);
								}}
								onMouseUp={finishDrag}
								onMouseLeave={() => setDrag(null)}
								onDoubleClick={showControls ? resetZoom : undefined}
							>
								<defs>
									<linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
										<stop
											offset="0%"
											stopColor={FORECAST_COLOR}
											stopOpacity={0.3}
										/>
										<stop
											offset="100%"
											stopColor={FORECAST_COLOR}
											stopOpacity={0.06}
										/>
									</linearGradient>
								</defs>
								<CartesianGrid
									vertical={false}
									stroke="var(--border)"
									strokeOpacity={0.5}
								/>
								{cutoff !== undefined && domain && cutoff > domain[0] && (
									<ReferenceArea
										x1={domain[0]}
										x2={Math.min(cutoff, domain[1])}
										fill="var(--muted-foreground)"
										fillOpacity={0.08}
									/>
								)}
								<XAxis
									type="number"
									dataKey="timestamp"
									allowDataOverflow
									domain={domain}
									tickFormatter={(time: number) =>
										formatForecastTick(time, domain ? domain[1] - domain[0] : 0)
									}
									tick={{ fill: "var(--muted-foreground)", fontSize: 11 }}
									axisLine={false}
									tickLine={false}
									minTickGap={50}
									tickMargin={12}
								/>
								<YAxis
									tickFormatter={formatForecastNumber}
									tick={{ fill: "var(--muted-foreground)", fontSize: 11 }}
									axisLine={false}
									tickLine={false}
									width={timelineGutter}
									tickMargin={12}
									domain={["auto", "auto"]}
								/>
								<Tooltip
									content={<ForecastTooltip pointStatistic={pointStatistic} />}
									cursor={{
										stroke: "var(--muted-foreground)",
										strokeDasharray: "3 3",
									}}
								/>
								{cutoffVisible && (
									<ReferenceLine
										x={cutoff}
										stroke="var(--muted-foreground)"
										label={{
											value: backtest ? "Holdout cutoff" : "History ends",
											position: "insideTopRight",
											fill: "var(--muted-foreground)",
											fontSize: 10,
										}}
									/>
								)}
								{drag && (
									<ReferenceArea
										x1={Math.min(...drag)}
										x2={Math.max(...drag)}
										fill="var(--primary)"
										fillOpacity={0.15}
										stroke="var(--primary)"
									/>
								)}
								{levels.length > 1 && (
									<Area
										type="linear"
										dataKey="interval"
										stroke="none"
										fill={`url(#${gradientId})`}
										isAnimationActive={false}
										connectNulls={false}
									/>
								)}
								{visibleTransition && (
									<ReferenceLine
										segment={visibleTransition}
										className="forecast-transition"
										stroke={FORECAST_COLOR}
										strokeWidth={2}
										strokeDasharray="3 3"
										strokeOpacity={0.7}
										aria-label="Transition from last observation to first forecast"
									/>
								)}
								<Line
									type="linear"
									dataKey="history"
									stroke="var(--foreground)"
									strokeWidth={1.8}
									dot={false}
									isAnimationActive={false}
									connectNulls={false}
								/>
								<Line
									type="linear"
									dataKey="actual"
									stroke="var(--muted-foreground)"
									strokeWidth={1.5}
									strokeDasharray="4 4"
									dot={false}
									isAnimationActive={false}
									connectNulls={false}
								/>
								<Line
									type="linear"
									dataKey="forecast"
									stroke={FORECAST_COLOR}
									strokeWidth={2}
									dot={false}
									isAnimationActive={false}
									connectNulls={false}
								/>
							</ComposedChart>
						</ResponsiveContainer>
					) : (
						<div className="absolute inset-x-6 inset-y-8 flex flex-col items-center justify-center rounded-xl border border-dashed border-border bg-muted/20 px-6 text-center">
							<Activity className="mb-3 size-7 text-muted-foreground" />
							<p className="text-sm text-foreground">
								See where your data is heading
							</p>
							<p className="mt-1 max-w-sm text-xs leading-relaxed text-muted-foreground">
								Choose a table, map a timestamp and a metric, then run your
								forecasting model.
							</p>
						</div>
					)}
				</div>
				{showControls && canZoom && domain && (
					<div
						className={cn(
							"space-y-2 border-t py-3",
							compact ? "px-3" : "px-4 sm:px-5",
						)}
					>
						<div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 text-xs text-muted-foreground">
							<span aria-live="polite" aria-label="Visible time range">
								{formatForecastTime(domain[0], true)} to{" "}
								{formatForecastTime(domain[1], true)} UTC
							</span>
							<span>Drag the chart to zoom, or move the range handles.</span>
						</div>
						<div className="relative h-12 rounded-md border bg-muted/20">
							<div
								className="pointer-events-none absolute inset-1"
								aria-hidden="true"
							>
								<ResponsiveContainer
									width="100%"
									height="100%"
									minWidth={0}
									initialDimension={{ width: 600, height: 40 }}
								>
									<LineChart
										data={points}
										margin={{ top: 2, right: 0, bottom: 2, left: 0 }}
										accessibilityLayer={false}
									>
										<XAxis
											hide
											dataKey="timestamp"
											type="number"
											domain={["dataMin", "dataMax"]}
										/>
										<YAxis hide domain={["auto", "auto"]} />
										{transition && (
											<ReferenceLine
												segment={transition}
												className="forecast-transition"
												stroke={FORECAST_COLOR}
												strokeWidth={1.5}
												strokeDasharray="3 3"
												strokeOpacity={0.7}
											/>
										)}
										<Line
											dataKey="history"
											stroke="var(--muted-foreground)"
											strokeWidth={1}
											dot={false}
											isAnimationActive={false}
										/>
										<Line
											dataKey="forecast"
											stroke={FORECAST_COLOR}
											strokeWidth={1.5}
											dot={false}
											isAnimationActive={false}
										/>
									</LineChart>
								</ResponsiveContainer>
							</div>
							<SliderPrimitive.Root
								min={0}
								max={lastIndex}
								step={1}
								minStepsBetweenThumbs={1}
								value={range}
								onValueChange={(value) => setRange([value[0], value[1]])}
								className="relative flex h-full w-full touch-none select-none items-center"
								aria-label="Chart visible range"
							>
								<SliderPrimitive.Track className="relative h-full grow overflow-hidden rounded-md">
									<SliderPrimitive.Range className="absolute h-full border-x border-primary/50 bg-primary/10" />
								</SliderPrimitive.Track>
								{range.map((index, handle) => (
									<SliderPrimitive.Thumb
										key={handle === 0 ? "start" : "end"}
										aria-label={
											handle === 0 ? "Visible range start" : "Visible range end"
										}
										aria-valuetext={`${formatForecastTime(points[index].timestamp, true)} UTC`}
										className="flex h-12 w-4 cursor-ew-resize items-center justify-center rounded border border-primary bg-background shadow-sm ring-ring focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-offset-2 focus-visible:ring-offset-background"
									>
										<span className="h-4 w-0.5 rounded-full bg-primary" />
									</SliderPrimitive.Thumb>
								))}
							</SliderPrimitive.Root>
						</div>
					</div>
				)}
				{covariates.length > 0 && (
					<div className="border-t border-border">
						<div
							className={cn(
								"flex items-center justify-between gap-3 py-3 text-xs font-medium uppercase tracking-[0.16em] text-muted-foreground",
								compact ? "px-3" : "px-4 sm:px-5",
							)}
						>
							<span>Covariates</span>
							<span>Additional model inputs</span>
						</div>
						{covariates.map((covariate) => (
							<div
								key={covariate.name}
								className={cn(
									"grid items-center border-t border-border/50 py-2.5",
									compact ? "px-3" : "px-4 sm:px-5",
									!covariate.enabled && "opacity-50",
								)}
								style={{
									gridTemplateColumns: `${timelineGutter}px minmax(0, 1fr) 48px`,
								}}
							>
								<div className="min-w-0">
									<p
										className="truncate text-xs leading-normal text-foreground"
										title={covariate.name}
									>
										{covariate.name}
									</p>
									<p className="mt-0.5 text-xs leading-normal text-muted-foreground">
										{covariate.role === "known"
											? "Known future"
											: "Past values"}
									</p>
								</div>
								<div className="h-8 min-w-0" aria-hidden="true">
									{covariate.points.length > 0 ? (
										<ResponsiveContainer
											width="100%"
											height="100%"
											minWidth={0}
											initialDimension={{ width: 300, height: 32 }}
										>
											<LineChart
												accessibilityLayer={false}
												data={
													domain
														? covariate.points.filter(
																(point) =>
																	point.timestamp >= domain[0] &&
																	point.timestamp <= domain[1],
															)
														: covariate.points
												}
												margin={{ left: 0, right: 0, top: 0, bottom: 0 }}
											>
												<XAxis
													hide
													type="number"
													dataKey="timestamp"
													allowDataOverflow
													domain={domain ?? ["dataMin", "dataMax"]}
												/>
												<YAxis hide domain={["auto", "auto"]} />
												{cutoff !== undefined &&
													domain &&
													cutoff > domain[0] && (
														<ReferenceArea
															x1={domain[0]}
															x2={Math.min(cutoff, domain[1])}
															fill="var(--muted-foreground)"
															fillOpacity={0.08}
														/>
													)}
												{cutoffVisible && (
													<ReferenceLine
														x={cutoff}
														stroke="var(--muted-foreground)"
													/>
												)}
												<Line
													dataKey="value"
													type="linear"
													stroke={
														covariate.enabled
															? "var(--foreground)"
															: "var(--muted-foreground)"
													}
													strokeWidth={1.2}
													dot={false}
													connectNulls={false}
													isAnimationActive={false}
												/>
											</LineChart>
										</ResponsiveContainer>
									) : (
										<div className="flex h-full items-center text-xs text-muted-foreground">
											Run to view values
										</div>
									)}
								</div>
								{readOnly ? (
									<span className="justify-self-end text-xs text-muted-foreground">
										Included
									</span>
								) : (
									<Switch
										checked={covariate.enabled}
										onCheckedChange={(enabled) =>
											onCovariateToggle?.(covariate.name, enabled)
										}
										disabled={
											!onCovariateToggle ||
											(!canUseCovariates && !covariate.enabled) ||
											covariate.disabled ||
											loading
										}
										aria-label={`Use ${covariate.name} as a covariate`}
										className="justify-self-end"
									/>
								)}
							</div>
						))}
					</div>
				)}
				{hasEvaluationMetrics && !fullscreen && (
					<p
						className={cn(
							"border-t border-border py-3 text-xs leading-relaxed text-muted-foreground",
							compact ? "px-3" : "px-4 sm:px-5",
						)}
						aria-label="Evaluation scope"
					>
						Holdout metrics below measure {metric} for{" "}
						{selectedSeries || "this series"}, using its held-out actual values.
					</p>
				)}
				{stale && (
					<p className="border-t border-border px-6 py-3 text-xs text-muted-foreground">
						{failedRefresh
							? "The latest run failed. Showing the previous successful forecast."
							: "This chart belongs to the previous run. Run again to apply your changes."}
					</p>
				)}
			</section>
		);
	};
	return (
		<Dialog
			open={expanded && showControls}
			onOpenChange={(open) => {
				setExpanded(open);
				setDrag(null);
			}}
		>
			{renderChart(false)}
			<DialogContent
				showCloseButton={false}
				className="h-dvh max-h-dvh w-screen max-w-none gap-0 rounded-none border-0 bg-card p-0 sm:max-w-none [&_[data-slot=dialog-accent]]:hidden"
			>
				<DialogTitle className="sr-only">
					{metric || "Forecast"} {backtest ? "holdout evaluation" : "forecast"}
				</DialogTitle>
				<DialogDescription className="sr-only">
					Explore the timeline with zoom controls and range handles. Press
					Escape to exit full screen.
				</DialogDescription>
				{renderChart(true)}
			</DialogContent>
		</Dialog>
	);
}

function ForecastTooltip({
	active,
	payload,
	pointStatistic,
}: {
	active?: boolean;
	payload?: readonly { payload?: ForecastPlotPoint }[];
	pointStatistic: "mean" | "median";
}) {
	const point = payload?.[0]?.payload;
	if (!active || !point) return null;
	return (
		<div className="min-w-40 rounded-lg border border-border bg-popover p-3 text-xs text-popover-foreground shadow-xl">
			<p className="mb-2 text-xs text-muted-foreground">
				{formatForecastTime(point.timestamp, true)} UTC
			</p>
			{point.history !== null && (
				<TooltipValue label="Observed" value={point.history} />
			)}
			{point.actual !== null && (
				<TooltipValue label="Held-out actual" value={point.actual} />
			)}
			{point.forecast !== null && (
				<TooltipValue
					label={`${pointStatistic === "mean" ? "Mean" : "Median"} forecast`}
					value={point.forecast}
					accent
				/>
			)}
			{point.interval && (
				<div className="mt-1 flex justify-between gap-5 text-muted-foreground">
					<span>Quantile range</span>
					<span>
						{formatForecastNumber(point.interval[0])} –{" "}
						{formatForecastNumber(point.interval[1])}
					</span>
				</div>
			)}
		</div>
	);
}

function TooltipValue({
	label,
	value,
	accent,
}: { label: string; value: number; accent?: boolean }) {
	return (
		<div
			className={cn(
				"mt-1 flex justify-between gap-5",
				accent && "text-chart-1",
			)}
		>
			<span>{label}</span>
			<span className="font-medium tabular-nums">
				{formatForecastNumber(value)}
			</span>
		</div>
	);
}

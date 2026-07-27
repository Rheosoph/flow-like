"use client";

import { useQuery } from "@tanstack/react-query";
import { BarChart3, Download, Table2, TriangleAlert } from "lucide-react";
import { useId, useMemo } from "react";
import {
	Area,
	AreaChart,
	Bar,
	BarChart,
	CartesianGrid,
	Line,
	LineChart,
	XAxis,
	YAxis,
} from "recharts";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import {
	Button,
	type ChartConfig,
	ChartContainer,
	ChartLegend,
	ChartLegendContent,
	ChartTooltip,
	ChartTooltipContent,
	Skeleton,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../../../ui";
import {
	type ITelemetryQueryLayout,
	type ITelemetryQueryRequest,
	type ITelemetryQueryResponse,
	type ITelemetryQueryValue,
	type ITelemetryQueryView,
	TELEMETRY_QUERY_MAX_ROWS,
	TELEMETRY_QUERY_TABLE_PREVIEW_ROWS,
	formatTelemetryQueryValue,
	normalizeTelemetryQuery,
	telemetryQueryFileName,
	telemetryQueryLayout,
	telemetryQueryTruncated,
	telemetryRowsToCsv,
} from "./query-types";
import {
	EmptyState,
	type TelemetryBucket,
	formatBucketTick,
} from "./telemetry-shared";

const QUERY_SERIES_COLORS = [
	"var(--chart-1)",
	"var(--chart-2)",
	"var(--chart-3)",
	"var(--chart-4)",
	"var(--chart-5)",
] as const;

const MAX_CHART_SERIES = QUERY_SERIES_COLORS.length;

const MAX_CHART_BARS = 20;

const OTHER_SERIES_LABEL = "Other";

const singleSeriesChartConfig = {
	value: { label: "Value", color: QUERY_SERIES_COLORS[0] },
} satisfies ChartConfig;

function seriesSlot(index: number) {
	return `s${index}`;
}

function bucketForInterval(
	interval: ITelemetryQueryRequest["interval"],
): TelemetryBucket {
	if (interval === "minute") return "minute";
	if (interval === "day") return "day";
	return "hour";
}

function toNumber(value: ITelemetryQueryValue): number {
	const parsed = typeof value === "number" ? value : Number(value);
	return Number.isFinite(parsed) ? parsed : 0;
}

function toKey(value: ITelemetryQueryValue): string {
	if (value === null || value === undefined || value === "") return "—";
	return String(value);
}

export function useTelemetryQueryResult(
	profile: IProfile | undefined,
	request: ITelemetryQueryRequest | null,
	enabled = true,
) {
	const backend = useBackend();
	const normalized = useMemo(
		() => (request ? normalizeTelemetryQuery(request) : null),
		[request],
	);
	return useQuery<ITelemetryQueryResponse>({
		queryKey: ["admin", "telemetry", "query", JSON.stringify(normalized)],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			if (!normalized) throw new Error("No query configured");
			return backend.apiState.post<ITelemetryQueryResponse>(
				profile,
				"admin/telemetry/query",
				normalized,
			);
		},
		enabled: Boolean(profile) && Boolean(normalized) && enabled,
	});
}

export function downloadTelemetryQueryCsv(
	name: string,
	response: ITelemetryQueryResponse,
) {
	const csv = telemetryRowsToCsv(response.columns, response.rows);
	const blob = new Blob([csv], { type: "text/csv;charset=utf-8;" });
	const url = URL.createObjectURL(blob);
	const anchor = document.createElement("a");
	anchor.href = url;
	anchor.download = telemetryQueryFileName(
		name,
		telemetryQueryTruncated(response),
	);
	document.body.appendChild(anchor);
	anchor.click();
	anchor.remove();
	URL.revokeObjectURL(url);
}

/** The row cap silently drops part of the range — say so above the result. */
function TruncationNotice({ compact }: { readonly compact: boolean }) {
	return (
		<div className="flex items-start gap-2 rounded-lg border border-amber-500/50 bg-amber-500/10 px-3 py-2 text-amber-700 dark:text-amber-400">
			<TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
			<div className={compact ? "text-[11px]" : "text-xs"}>
				<span className="font-semibold">
					Capped at {TELEMETRY_QUERY_MAX_ROWS.toLocaleString()} rows.
				</span>{" "}
				Part of the selected range is missing from this result and from the CSV
				export. Narrow the time range, pick a coarser interval, or drop the
				breakdown.
			</div>
		</div>
	);
}

interface SeriesChartModel {
	data: Record<string, string | number>[];
	config: ChartConfig;
	slots: string[];
}

function buildTimeseriesModel(
	response: ITelemetryQueryResponse,
	layout: ITelemetryQueryLayout,
): SeriesChartModel {
	if (layout.breakdownIndex < 0) {
		return {
			data: response.rows.map((row) => ({
				ts: toKey(row[layout.tsIndex]),
				value: toNumber(row[layout.valueIndex]),
			})),
			config: singleSeriesChartConfig,
			slots: ["value"],
		};
	}

	const totals = new Map<string, number>();
	for (const row of response.rows) {
		const key = toKey(row[layout.breakdownIndex]);
		totals.set(key, (totals.get(key) ?? 0) + toNumber(row[layout.valueIndex]));
	}

	const ranked = [...totals.entries()]
		.sort((a, b) => b[1] - a[1])
		.map(([key]) => key);
	const overflow = ranked.length > MAX_CHART_SERIES;
	const named = (
		overflow ? ranked.slice(0, MAX_CHART_SERIES - 1) : ranked
	).sort((a, b) => a.localeCompare(b));
	const labels = overflow ? [...named, OTHER_SERIES_LABEL] : named;
	const slotByKey = new Map<string, string>();
	named.forEach((key, index) => slotByKey.set(key, seriesSlot(index)));
	const otherSlot = seriesSlot(MAX_CHART_SERIES - 1);

	const buckets = new Map<string, Record<string, string | number>>();
	for (const row of response.rows) {
		const ts = toKey(row[layout.tsIndex]);
		const key = toKey(row[layout.breakdownIndex]);
		const slot = slotByKey.get(key) ?? (overflow ? otherSlot : null);
		if (!slot) continue;
		let bucket = buckets.get(ts);
		if (!bucket) {
			bucket = { ts };
			buckets.set(ts, bucket);
		}
		bucket[slot] = Number(bucket[slot] ?? 0) + toNumber(row[layout.valueIndex]);
	}

	const config: ChartConfig = {};
	labels.forEach((label, index) => {
		const slot =
			overflow && index === labels.length - 1
				? otherSlot
				: (slotByKey.get(label) ?? seriesSlot(index));
		config[slot] = {
			label,
			color: QUERY_SERIES_COLORS[index % QUERY_SERIES_COLORS.length],
		};
	});

	return {
		data: [...buckets.values()],
		config,
		slots: Object.keys(config),
	};
}

function TimeseriesChart({
	response,
	layout,
	bucket,
	height,
}: {
	readonly response: ITelemetryQueryResponse;
	readonly layout: ITelemetryQueryLayout;
	readonly bucket: TelemetryBucket;
	readonly height: string;
}) {
	const gradientId = useId().replace(/:/g, "");
	const model = useMemo(
		() => buildTimeseriesModel(response, layout),
		[response, layout],
	);
	const multiSeries = model.slots.length > 1;

	if (model.data.length === 0) {
		return (
			<EmptyState
				message="No rows returned for this query."
				className={`${height} text-sm`}
			/>
		);
	}

	if (multiSeries) {
		return (
			<ChartContainer config={model.config} className={`${height} w-full`}>
				<LineChart
					data={model.data}
					margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
				>
					<CartesianGrid strokeDasharray="3 3" vertical={false} opacity={0.4} />
					<XAxis
						dataKey="ts"
						tickLine={false}
						axisLine={false}
						tick={{ fontSize: 11 }}
						tickFormatter={(v) => formatBucketTick(v as string, bucket)}
						minTickGap={32}
					/>
					<YAxis
						tickLine={false}
						axisLine={false}
						tick={{ fontSize: 11 }}
						width={56}
						tickFormatter={(v) => formatTelemetryQueryValue(Number(v))}
					/>
					<ChartTooltip
						cursor={{ stroke: "var(--border)", strokeDasharray: "3 3" }}
						content={
							<ChartTooltipContent
								indicator="dot"
								labelFormatter={(value) =>
									formatBucketTick(value as string, bucket)
								}
							/>
						}
					/>
					<ChartLegend
						content={({ payload, verticalAlign }) => (
							<ChartLegendContent
								payload={payload}
								verticalAlign={verticalAlign}
							/>
						)}
					/>
					{model.slots.map((slot) => (
						<Line
							key={slot}
							type="monotone"
							dataKey={slot}
							stroke={`var(--color-${slot})`}
							strokeWidth={2}
							dot={false}
							activeDot={{ r: 4 }}
							connectNulls
						/>
					))}
				</LineChart>
			</ChartContainer>
		);
	}

	return (
		<ChartContainer config={model.config} className={`${height} w-full`}>
			<AreaChart
				data={model.data}
				margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
			>
				<defs>
					<linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
						<stop
							offset="0%"
							stopColor="var(--color-value)"
							stopOpacity={0.3}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-value)"
							stopOpacity={0.03}
						/>
					</linearGradient>
				</defs>
				<CartesianGrid strokeDasharray="3 3" vertical={false} opacity={0.4} />
				<XAxis
					dataKey="ts"
					tickLine={false}
					axisLine={false}
					tick={{ fontSize: 11 }}
					tickFormatter={(v) => formatBucketTick(v as string, bucket)}
					minTickGap={32}
				/>
				<YAxis
					tickLine={false}
					axisLine={false}
					tick={{ fontSize: 11 }}
					width={56}
					tickFormatter={(v) => formatTelemetryQueryValue(Number(v))}
				/>
				<ChartTooltip
					cursor={{ stroke: "var(--border)", strokeDasharray: "3 3" }}
					content={
						<ChartTooltipContent
							indicator="dot"
							labelFormatter={(value) =>
								formatBucketTick(value as string, bucket)
							}
						/>
					}
				/>
				<Area
					type="monotone"
					dataKey="value"
					stroke="var(--color-value)"
					fill={`url(#${gradientId})`}
					strokeWidth={2}
					connectNulls
				/>
			</AreaChart>
		</ChartContainer>
	);
}

function BreakdownChart({
	response,
	layout,
	height,
}: {
	readonly response: ITelemetryQueryResponse;
	readonly layout: ITelemetryQueryLayout;
	readonly height: string;
}) {
	const data = useMemo(
		() =>
			response.rows
				.map((row) => ({
					key: toKey(row[layout.breakdownIndex]),
					value: toNumber(row[layout.valueIndex]),
				}))
				.sort((a, b) => b.value - a.value)
				.slice(0, MAX_CHART_BARS),
		[response.rows, layout],
	);

	if (data.length === 0) {
		return (
			<EmptyState
				message="No rows returned for this query."
				className={`${height} text-sm`}
			/>
		);
	}

	return (
		<ChartContainer
			config={singleSeriesChartConfig}
			className={`${height} w-full`}
		>
			<BarChart
				data={data}
				layout="vertical"
				margin={{ top: 4, right: 16, left: 0, bottom: 4 }}
			>
				<CartesianGrid strokeDasharray="3 3" horizontal={false} opacity={0.4} />
				<XAxis
					type="number"
					tickLine={false}
					axisLine={false}
					tick={{ fontSize: 11 }}
					tickFormatter={(v) => formatTelemetryQueryValue(Number(v))}
				/>
				<YAxis
					type="category"
					dataKey="key"
					tickLine={false}
					axisLine={false}
					tick={{ fontSize: 11 }}
					width={148}
				/>
				<ChartTooltip
					cursor={{ fill: "var(--muted)", opacity: 0.4 }}
					content={<ChartTooltipContent indicator="dot" />}
				/>
				<Bar
					dataKey="value"
					fill="var(--color-value)"
					radius={[0, 4, 4, 0]}
					barSize={14}
				/>
			</BarChart>
		</ChartContainer>
	);
}

function ScalarResult({
	response,
	layout,
}: {
	readonly response: ITelemetryQueryResponse;
	readonly layout: ITelemetryQueryLayout;
}) {
	const value = response.rows[0]?.[layout.valueIndex] ?? null;
	return (
		<div className="flex flex-col items-center justify-center rounded-lg border border-border bg-muted/40 py-10">
			<span className="text-[10px] uppercase tracking-wide text-muted-foreground">
				{response.columns[layout.valueIndex] ?? "value"}
			</span>
			<span className="mt-1 text-4xl font-bold tabular-nums">
				{formatTelemetryQueryValue(value)}
			</span>
		</div>
	);
}

function ResultTable({
	response,
}: { readonly response: ITelemetryQueryResponse }) {
	const rows = response.rows.slice(0, TELEMETRY_QUERY_TABLE_PREVIEW_ROWS);
	const hidden = response.rows.length - rows.length;
	const capped = telemetryQueryTruncated(response);

	if (response.rows.length === 0) {
		return (
			<EmptyState
				message="No rows returned for this query."
				className="py-10 text-sm"
			/>
		);
	}

	return (
		<div className="space-y-2">
			<div className="max-h-[28rem] overflow-auto rounded-lg border">
				<Table>
					<TableHeader>
						<TableRow>
							{response.columns.map((column) => (
								<TableHead key={column} className="whitespace-nowrap">
									{column}
								</TableHead>
							))}
						</TableRow>
					</TableHeader>
					<TableBody>
						{rows.map((row, rowIndex) => (
							<TableRow key={`${rowIndex}-${String(row[0] ?? "")}`}>
								{response.columns.map((column, cellIndex) => (
									<TableCell
										key={column}
										className={`whitespace-nowrap text-xs ${
											typeof row[cellIndex] === "number"
												? "text-right tabular-nums"
												: "font-mono"
										}`}
									>
										{formatTelemetryQueryValue(row[cellIndex] ?? null)}
									</TableCell>
								))}
							</TableRow>
						))}
					</TableBody>
				</Table>
			</div>
			{hidden > 0 ? (
				<p className="text-[11px] text-muted-foreground">
					Showing the first {rows.length.toLocaleString()} of the{" "}
					{response.rows.length.toLocaleString()} rows the server returned. The
					CSV download contains all {response.rows.length.toLocaleString()} of
					them
					{capped
						? " — but the server already capped the query, so neither this table nor the CSV covers the full range."
						: "."}
				</p>
			) : null}
		</div>
	);
}

export interface TelemetryQueryResultViewProps {
	request: ITelemetryQueryRequest;
	response?: ITelemetryQueryResponse;
	loading?: boolean;
	error?: Error | null;
	view: ITelemetryQueryView;
	onViewChange?: (view: ITelemetryQueryView) => void;
	name?: string;
	compact?: boolean;
	showToolbar?: boolean;
}

export function TelemetryQueryResultView({
	request,
	response,
	loading,
	error,
	view,
	onViewChange,
	name = "telemetry-query",
	compact = false,
	showToolbar = true,
}: Readonly<TelemetryQueryResultViewProps>) {
	const layout = useMemo(
		() => telemetryQueryLayout(request, response),
		[request, response],
	);
	const height = compact ? "h-56" : "h-72";
	const bucket = bucketForInterval(request.interval);
	const chartable = layout.kind !== "scalar";
	const capped = telemetryQueryTruncated(response);

	if (loading) {
		return (
			<div className="space-y-2">
				{showToolbar ? <Skeleton className="h-8 w-full" /> : null}
				<Skeleton className={`${height} w-full`} />
			</div>
		);
	}

	if (error) {
		return (
			<div className="flex items-center gap-2 rounded-lg border border-dashed border-destructive/50 px-3 py-6 text-xs text-destructive">
				<TriangleAlert className="h-4 w-4 shrink-0" />
				<span className="truncate">{error.message}</span>
			</div>
		);
	}

	if (!response) {
		return (
			<EmptyState
				message="Run the query to see results."
				className={`${height} text-sm`}
			/>
		);
	}

	return (
		<div className="space-y-2">
			{showToolbar ? (
				<div className="flex flex-wrap items-center justify-between gap-2">
					<span className="text-[11px] tabular-nums text-muted-foreground">
						{response.total.toLocaleString()} rows ·{" "}
						{response.interval === "none" ? "no buckets" : response.interval}
					</span>
					<div className="flex items-center gap-1">
						{chartable && onViewChange ? (
							<>
								<Button
									variant={view === "chart" ? "secondary" : "ghost"}
									size="sm"
									onClick={() => onViewChange("chart")}
								>
									<BarChart3 className="mr-1 h-3.5 w-3.5" />
									Chart
								</Button>
								<Button
									variant={view === "table" ? "secondary" : "ghost"}
									size="sm"
									onClick={() => onViewChange("table")}
								>
									<Table2 className="mr-1 h-3.5 w-3.5" />
									Table
								</Button>
							</>
						) : null}
						<Button
							variant="outline"
							size="sm"
							disabled={response.rows.length === 0}
							onClick={() => downloadTelemetryQueryCsv(name, response)}
						>
							<Download className="mr-1 h-3.5 w-3.5" />
							CSV
						</Button>
					</div>
				</div>
			) : null}

			{capped ? <TruncationNotice compact={compact} /> : null}

			{view === "table" || !chartable ? (
				layout.kind === "scalar" && view !== "table" ? (
					<ScalarResult response={response} layout={layout} />
				) : (
					<ResultTable response={response} />
				)
			) : layout.kind === "breakdown" ? (
				<BreakdownChart response={response} layout={layout} height={height} />
			) : (
				<TimeseriesChart
					response={response}
					layout={layout}
					bucket={bucket}
					height={height}
				/>
			)}
		</div>
	);
}

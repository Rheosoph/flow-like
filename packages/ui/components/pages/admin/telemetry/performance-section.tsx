"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { Gauge } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Area, AreaChart, CartesianGrid, XAxis, YAxis } from "recharts";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	type ChartConfig,
	ChartContainer,
	ChartTooltip,
	ChartTooltipContent,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../../../ui";
import {
	DAILY_PERCENTILE_HINT,
	TelemetryGranularityNotice,
	UNAVAILABLE_PERCENTILE_HINT,
	approximateValue,
	isDailyGranularity,
} from "./granularity-notice";
import {
	EmptyState,
	type TelemetryBucket,
	formatBucketTick,
	trendBucketForHours,
} from "./telemetry-shared";
import {
	PERF_METRIC_ORDER,
	RatingBadge,
	formatMetricValue,
	isUnitlessMetric,
	perfMetricLabel,
	ratingTone,
} from "./traces-shared";
import type {
	ITelemetryPerfMetricSummary,
	ITelemetryPerfPathRow,
	ITelemetryPerfTrendPoint,
	ITelemetryPerformanceResponse,
} from "./types";

const perfChartConfig = {
	p75: {
		label: "p75",
		color: "var(--chart-1)",
	},
} satisfies ChartConfig;

function metricRank(metric: string) {
	const index = PERF_METRIC_ORDER.indexOf(metric);
	return index === -1 ? PERF_METRIC_ORDER.length : index;
}

function formatAxisValue(metric: string, value: number) {
	if (isUnitlessMetric(metric)) return value.toFixed(2);
	if (value >= 1000) return `${(value / 1000).toFixed(1)}s`;
	return `${Math.round(value)}ms`;
}

/** Percentiles can be absent once a window is answered from daily rollups. */
function percentileLabel(
	metric: string,
	name: string,
	value: number,
	daily: boolean,
) {
	if (!Number.isFinite(value)) return `${name} n/a`;
	return `${name} ${approximateValue(formatMetricValue(metric, value), daily)}`;
}

function VitalTile({
	summary,
	daily,
}: {
	readonly summary: ITelemetryPerfMetricSummary;
	readonly daily: boolean;
}) {
	const tone = ratingTone(summary.rating);
	const p75Available = Number.isFinite(summary.p75);
	const p95Available = Number.isFinite(summary.p95);
	return (
		<div className={`rounded-xl border p-4 ${tone.tile}`}>
			<div className="flex items-center justify-between gap-2 text-muted-foreground">
				<span className="truncate text-xs uppercase tracking-wide">
					{summary.metric}
				</span>
				<span className="truncate text-[11px]">
					{perfMetricLabel(summary.metric)}
				</span>
			</div>
			<div className="mt-1 truncate text-2xl font-bold tabular-nums">
				{p75Available
					? approximateValue(
							formatMetricValue(summary.metric, summary.p75),
							daily,
						)
					: "n/a"}
			</div>
			<div className="mt-2 flex flex-wrap items-center gap-2">
				<RatingBadge rating={summary.rating} />
				<span className="text-[11px] tabular-nums text-muted-foreground">
					{summary.count.toLocaleString()} samples
				</span>
			</div>
			<div
				className="mt-1 text-[11px] tabular-nums text-muted-foreground"
				title={
					p95Available
						? daily
							? DAILY_PERCENTILE_HINT
							: undefined
						: UNAVAILABLE_PERCENTILE_HINT
				}
			>
				{percentileLabel(summary.metric, "p50", summary.p50, daily)} ·{" "}
				{percentileLabel(summary.metric, "p95", summary.p95, daily)}
			</div>
		</div>
	);
}

function PerfTrendChart({
	metric,
	points,
	bucket,
}: {
	readonly metric: string;
	readonly points: ITelemetryPerfTrendPoint[];
	readonly bucket: TelemetryBucket;
}) {
	const data = useMemo(
		() => points.map((point) => ({ ts: point.ts, p75: point.p75 })),
		[points],
	);

	if (data.length === 0) {
		return (
			<EmptyState
				message="No samples for this metric in the selected window."
				className="h-64 text-sm"
			/>
		);
	}

	return (
		<ChartContainer config={perfChartConfig} className="h-64 w-full">
			<AreaChart data={data} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
				<defs>
					<linearGradient id="telemetryPerfP75" x1="0" y1="0" x2="0" y2="1">
						<stop offset="0%" stopColor="var(--color-p75)" stopOpacity={0.3} />
						<stop
							offset="100%"
							stopColor="var(--color-p75)"
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
					tickFormatter={(v) => formatAxisValue(metric, Number(v))}
					width={52}
				/>
				<ChartTooltip
					cursor={{ stroke: "var(--border)", strokeDasharray: "3 3" }}
					content={
						<ChartTooltipContent
							indicator="dot"
							labelFormatter={(value) =>
								formatBucketTick(value as string, bucket)
							}
							formatter={(value) => (
								<span className="text-xs text-muted-foreground">
									p75{" "}
									<span className="font-medium tabular-nums text-foreground">
										{formatMetricValue(metric, Number(value))}
									</span>
								</span>
							)}
						/>
					}
				/>
				<Area
					type="monotone"
					dataKey="p75"
					stroke="var(--color-p75)"
					fill="url(#telemetryPerfP75)"
					strokeWidth={2}
					connectNulls
				/>
			</AreaChart>
		</ChartContainer>
	);
}

function PathTable({ rows }: { readonly rows: ITelemetryPerfPathRow[] }) {
	const { t } = useTranslation("admin");
	const sorted = useMemo(() => [...rows].sort((a, b) => b.p75 - a.p75), [rows]);

	if (sorted.length === 0) {
		return (
			<EmptyState
				message="No path-level samples reported in the selected window."
				className="m-4 py-10 text-sm"
			/>
		);
	}

	return (
		<Table>
			<TableHeader>
				<TableRow>
					<TableHead>{t("path", "Path")}</TableHead>
					<TableHead>{t("metric", "Metric")}</TableHead>
					<TableHead className="text-right">p75</TableHead>
					<TableHead className="text-right">
						{t("samples", "Samples")}
					</TableHead>
				</TableRow>
			</TableHeader>
			<TableBody>
				{sorted.map((row) => (
					<TableRow key={`${row.path}:${row.metric}`}>
						<TableCell className="max-w-[24rem]">
							<span
								className="block truncate font-mono text-xs"
								title={row.path}
							>
								{row.path}
							</span>
						</TableCell>
						<TableCell className="font-mono text-xs uppercase text-muted-foreground">
							{row.metric}
						</TableCell>
						<TableCell className="text-right text-xs tabular-nums">
							{formatMetricValue(row.metric, row.p75)}
						</TableCell>
						<TableCell className="text-right text-xs tabular-nums text-muted-foreground">
							{row.count.toLocaleString()}
						</TableCell>
					</TableRow>
				))}
			</TableBody>
		</Table>
	);
}

interface PerformanceSectionProps {
	profile: IProfile | undefined;
	hours: number;
	source?: string;
}

export function PerformanceSection({
	profile,
	hours,
	source,
}: Readonly<PerformanceSectionProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const [metric, setMetric] = useState<string | null>(null);

	const performance = useQuery<ITelemetryPerformanceResponse>({
		queryKey: ["admin", "telemetry", "performance", hours, source ?? "all"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			const params = new URLSearchParams({ hours: String(hours) });
			if (source && source !== "all") params.set("source", source);
			return backend.apiState.get<ITelemetryPerformanceResponse>(
				profile,
				`admin/telemetry/performance?${params.toString()}`,
			);
		},
		enabled: !!profile,
	});

	const metrics = useMemo(
		() =>
			[...(performance.data?.metrics ?? [])].sort(
				(a, b) => metricRank(a.metric) - metricRank(b.metric),
			),
		[performance.data?.metrics],
	);

	useEffect(() => {
		if (metrics.length === 0) return;
		if (metric && metrics.some((m) => m.metric === metric)) return;
		setMetric(metrics[0].metric);
	}, [metric, metrics]);

	const activeMetric = metric ?? metrics[0]?.metric ?? "lcp";
	const trend = useMemo(
		() =>
			(performance.data?.trend ?? []).filter(
				(point) => point.metric === activeMetric,
			),
		[activeMetric, performance.data?.trend],
	);
	const paths = useMemo(
		() =>
			(performance.data?.byPath ?? []).filter(
				(row) => row.metric === activeMetric,
			),
		[activeMetric, performance.data?.byPath],
	);

	const bucket = trendBucketForHours(performance.data?.hours ?? hours);
	const daily = isDailyGranularity(performance.data);

	return (
		<section className="space-y-4">
			<h2 className="flex items-center gap-2 text-xl font-semibold">
				<Gauge className="h-5 w-5 text-primary" />
				{t("performance", "Performance")}
				<TelemetryGranularityNotice response={performance.data} />
			</h2>

			{performance.isLoading ? (
				<div className="space-y-4">
					<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
						{["lcp", "inp", "cls", "ttfb"].map((key) => (
							<Skeleton key={key} className="h-32" />
						))}
					</div>
					<Skeleton className="h-64 w-full" />
					<Skeleton className="h-40 w-full" />
				</div>
			) : metrics.length === 0 ? (
				<EmptyState
					message="No performance samples in this window — web vitals appear once installs opt in to usage telemetry."
					className="py-10 text-sm"
				/>
			) : (
				<>
					<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
						{metrics.map((summary) => (
							<VitalTile key={summary.metric} summary={summary} daily={daily} />
						))}
					</div>

					<Card>
						<CardHeader className="pb-3">
							<div className="flex flex-wrap items-start justify-between gap-2">
								<div className="space-y-1">
									<CardTitle className="text-base">
										{t("p75OverTime", "p75 over time")}
									</CardTitle>
									<CardDescription>
										{t("75thPercentileOf", "75th percentile of")}{" "}
										<span className="font-mono">{activeMetric}</span>
										{t("bucketedBy", ", bucketed by")}{" "}
										<span className="font-mono">{bucket}</span>.
									</CardDescription>
								</div>
								<Select
									value={activeMetric}
									onValueChange={(v) => setMetric(v)}
								>
									<SelectTrigger className="w-56">
										<SelectValue placeholder="Metric" />
									</SelectTrigger>
									<SelectContent>
										{metrics.map((summary) => (
											<SelectItem key={summary.metric} value={summary.metric}>
												{`${summary.metric} —`}
												{perfMetricLabel(summary.metric)}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>
						</CardHeader>
						<CardContent>
							<PerfTrendChart
								metric={activeMetric}
								points={trend}
								bucket={bucket}
							/>
						</CardContent>
					</Card>

					<Card>
						<CardHeader className="pb-3">
							<CardTitle className="text-base">
								{t("byPath", "By path")}
							</CardTitle>
							<CardDescription>
								{t("slowestPathsFor", "Slowest paths for")}{" "}
								<span className="font-mono">{activeMetric}</span>
								{t("worstFirst", ", worst first.")}
							</CardDescription>
						</CardHeader>
						<CardContent className="p-0">
							<PathTable rows={paths} />
						</CardContent>
					</Card>
				</>
			)}
		</section>
	);
}

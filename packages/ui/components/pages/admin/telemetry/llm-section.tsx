"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	BrainCircuit,
	Coins,
	Gauge,
	MessagesSquare,
	Timer,
	TriangleAlert,
} from "lucide-react";
import { useMemo } from "react";
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
	AgentBackendsCard,
	RatePill,
	errorRateRating,
} from "./agent-backends-card";
import {
	DAILY_PERCENTILE_HINT,
	TelemetryGranularityNotice,
	UNAVAILABLE_PERCENTILE_HINT,
	approximateValue,
	isDailyGranularity,
} from "./granularity-notice";
import type {
	ITelemetryLlmModelRow,
	ITelemetryLlmResponse,
	ITelemetryLlmTrendPoint,
} from "./llm-types";
import { formatRatePercent } from "./release-health-section";
import {
	BarList,
	EmptyState,
	StatTile,
	type TelemetryBucket,
	formatBucketTick,
	trendBucketForHours,
} from "./telemetry-shared";
import { RatingBadge, formatDurationMs } from "./traces-shared";

const llmTrendChartConfig = {
	calls: {
		label: "Calls",
		color: "var(--chart-1)",
	},
	errors: {
		label: "Errors",
		color: "var(--destructive)",
	},
} satisfies ChartConfig;

function formatCompactNumber(value: number): string {
	if (!Number.isFinite(value)) return "—";
	const abs = Math.abs(value);
	if (abs >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
	if (abs >= 10_000) return `${(value / 1_000).toFixed(1)}k`;
	return value.toLocaleString();
}

function LlmTrendChart({
	points,
	bucket,
}: {
	readonly points: ITelemetryLlmTrendPoint[];
	readonly bucket: TelemetryBucket;
}) {
	const data = useMemo(
		() =>
			points.map((point) => ({
				ts: point.ts,
				calls: point.calls,
				errors: point.errors,
			})),
		[points],
	);

	if (data.length === 0) {
		return (
			<EmptyState
				message="No LLM calls in the selected window."
				className="h-64 text-sm"
			/>
		);
	}

	return (
		<ChartContainer config={llmTrendChartConfig} className="h-64 w-full">
			<AreaChart data={data} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
				<defs>
					<linearGradient id="telemetryLlmCalls" x1="0" y1="0" x2="0" y2="1">
						<stop
							offset="0%"
							stopColor="var(--color-calls)"
							stopOpacity={0.3}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-calls)"
							stopOpacity={0.03}
						/>
					</linearGradient>
					<linearGradient id="telemetryLlmErrors" x1="0" y1="0" x2="0" y2="1">
						<stop
							offset="0%"
							stopColor="var(--color-errors)"
							stopOpacity={0.3}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-errors)"
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
					allowDecimals={false}
					tickLine={false}
					axisLine={false}
					tick={{ fontSize: 11 }}
					width={44}
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
				<Area
					type="monotone"
					dataKey="calls"
					stroke="var(--color-calls)"
					fill="url(#telemetryLlmCalls)"
					strokeWidth={2}
				/>
				<Area
					type="monotone"
					dataKey="errors"
					stroke="var(--color-errors)"
					fill="url(#telemetryLlmErrors)"
					strokeWidth={2}
				/>
			</AreaChart>
		</ChartContainer>
	);
}

interface RateBarRow {
	key: string;
	label: string;
	count: number;
	errorRate: number;
}

function RateBarList({
	rows,
	emptyMessage,
}: {
	readonly rows: RateBarRow[];
	readonly emptyMessage: string;
}) {
	if (rows.length === 0) {
		return <EmptyState message={emptyMessage} />;
	}
	const max = Math.max(1, ...rows.map((row) => row.count));
	return (
		<ul className="space-y-1.5">
			{rows.map((row) => (
				<li
					key={row.key}
					className="flex items-center gap-2 rounded px-1 py-0.5"
				>
					<span
						className="w-28 truncate font-mono text-xs font-medium"
						title={row.label}
					>
						{row.label}
					</span>
					<div className="relative h-2 flex-1 overflow-hidden rounded-full bg-muted">
						<div
							className="h-full rounded-full bg-primary/60"
							style={{ width: `${(row.count / max) * 100}%` }}
						/>
					</div>
					<span className="w-14 text-right text-[11px] tabular-nums text-muted-foreground">
						{row.count.toLocaleString()}
					</span>
					<span className="w-20 text-right text-[11px] tabular-nums text-muted-foreground">
						{formatRatePercent(row.errorRate, 1)} err
					</span>
				</li>
			))}
		</ul>
	);
}

function ModelTable({ rows }: { readonly rows: ITelemetryLlmModelRow[] }) {
	const { t } = useTranslation("admin");
	const sorted = useMemo(
		() =>
			[...rows].sort(
				(a, b) => b.calls - a.calls || a.model.localeCompare(b.model),
			),
		[rows],
	);

	if (sorted.length === 0) {
		return (
			<EmptyState
				message="No model activity in the selected window."
				className="m-4 py-10 text-sm"
			/>
		);
	}

	return (
		<Table>
			<TableHeader>
				<TableRow>
					<TableHead>{t('model', 'Model')}</TableHead>
					<TableHead className="text-right">{t('calls', 'Calls')}</TableHead>
					<TableHead className="text-right">{t('errorRate', 'Error rate')}</TableHead>
					<TableHead className="text-right">{t('avg', 'Avg')}</TableHead>
					<TableHead className="text-right">p95</TableHead>
					<TableHead className="text-right">{t('tokens', 'Tokens')}</TableHead>
				</TableRow>
			</TableHeader>
			<TableBody>
				{sorted.map((row) => (
					<TableRow key={`${row.provider}:${row.model}`}>
						<TableCell className="max-w-[20rem]">
							<span
								className="block truncate font-mono text-xs font-medium"
								title={row.model}
							>
								{row.model}
							</span>
							<span className="font-mono text-[11px] text-muted-foreground">
								{row.provider}
							</span>
						</TableCell>
						<TableCell className="text-right text-xs tabular-nums">
							{row.calls.toLocaleString()}
						</TableCell>
						<TableCell className="text-right">
							<div className="flex justify-end">
								<RatePill rate={row.errorRate} kind="error" />
							</div>
						</TableCell>
						<TableCell className="text-right text-xs tabular-nums">
							{formatDurationMs(row.avgDurationMs)}
						</TableCell>
						<TableCell className="text-right text-xs tabular-nums">
							{formatDurationMs(row.p95DurationMs)}
						</TableCell>
						<TableCell className="text-right text-xs tabular-nums text-muted-foreground">
							{formatCompactNumber(row.totalTokens)}
						</TableCell>
					</TableRow>
				))}
			</TableBody>
		</Table>
	);
}

interface LlmSectionProps {
	profile: IProfile | undefined;
	hours: number;
	source?: string;
}

export function LlmSection({
	profile,
	hours,
	source,
}: Readonly<LlmSectionProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const llm = useQuery<ITelemetryLlmResponse>({
		queryKey: ["admin", "telemetry", "llm", hours, source ?? "all"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			const params = new URLSearchParams({ hours: String(hours) });
			if (source && source !== "all") params.set("source", source);
			return backend.apiState.get<ITelemetryLlmResponse>(
				profile,
				`admin/telemetry/llm?${params.toString()}`,
			);
		},
		enabled: !!profile,
	});

	const totals = llm.data?.totals;
	const bucket = trendBucketForHours(llm.data?.hours ?? hours);
	const modelCount = llm.data?.byModel.length ?? 0;
	const daily = isDailyGranularity(llm.data);
	const p95Available = Number.isFinite(totals?.p95DurationMs);

	const providerRows = useMemo<RateBarRow[]>(
		() =>
			(llm.data?.byProvider ?? []).map((row) => ({
				key: row.provider,
				label: row.provider,
				count: row.calls,
				errorRate: row.errorRate,
			})),
		[llm.data?.byProvider],
	);

	const operationRows = useMemo<RateBarRow[]>(
		() =>
			(llm.data?.byOperation ?? []).map((row) => ({
				key: row.operation,
				label: row.operation,
				count: row.calls,
				errorRate: row.errorRate,
			})),
		[llm.data?.byOperation],
	);

	const errorRows = useMemo(
		() =>
			(llm.data?.topErrors ?? []).map((row) => ({
				key: row.errorKind,
				label: row.errorKind,
				count: row.count,
			})),
		[llm.data?.topErrors],
	);

	return (
		<section className="space-y-4">
			<h2 className="flex items-center gap-2 text-xl font-semibold">
				<BrainCircuit className="h-5 w-5 text-primary" />
				{t('llmUsage', 'LLM usage')}
				<TelemetryGranularityNotice response={llm.data} />
			</h2>

			{llm.isLoading ? (
				<div className="space-y-4">
					<div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-5">
						{["calls", "errors", "avg", "p95", "tokens"].map((key) => (
							<Skeleton key={key} className="h-16" />
						))}
					</div>
					<Skeleton className="h-64 w-full" />
					<Skeleton className="h-40 w-full" />
				</div>
			) : !totals || totals.calls === 0 ? (
				<EmptyState
					message="No LLM calls in this window — model usage appears once installs opt in to usage telemetry."
					className="py-10 text-sm"
				/>
			) : (
				<>
					<div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-5">
						<StatTile
							label="Calls"
							value={totals.calls.toLocaleString()}
							icon={<MessagesSquare className="h-4 w-4" />}
							hint={`${modelCount.toLocaleString()} ${modelCount === 1 ? "model" : "models"}`}
						/>
						<StatTile
							label={t('errorRate', 'Error rate')}
							value={formatRatePercent(totals.errorRate, 1)}
							icon={<TriangleAlert className="h-4 w-4" />}
							extra={<RatingBadge rating={errorRateRating(totals.errorRate)} />}
							hint={t('valOfVal2CallsFailed', '{{val}} of {{val2}} calls failed', { val: totals.errors.toLocaleString(), val2: totals.calls.toLocaleString() })}
						/>
						<StatTile
							label={t('avgDuration', 'Avg duration')}
							value={formatDurationMs(totals.avgDurationMs)}
							icon={<Timer className="h-4 w-4" />}
							hint="Mean call latency"
						/>
						<StatTile
							label={t('p95Duration', 'p95 duration')}
							value={
								p95Available
									? approximateValue(
											formatDurationMs(totals.p95DurationMs),
											daily,
										)
									: "n/a"
							}
							icon={<Gauge className="h-4 w-4" />}
							hint={
								p95Available
									? daily
										? DAILY_PERCENTILE_HINT
										: t('95thPercentileLatency', '95th percentile latency')
									: UNAVAILABLE_PERCENTILE_HINT
							}
						/>
						<StatTile
							label={t('totalTokens', 'Total tokens')}
							value={formatCompactNumber(totals.totalTokens)}
							icon={<Coins className="h-4 w-4" />}
							hint={t('valPromptVal2Completion', '{{val}} prompt · {{val2}} completion', { val: formatCompactNumber(totals.promptTokens), val2: formatCompactNumber(totals.completionTokens) })}
						/>
					</div>

					<Card>
						<CardHeader className="pb-3">
							<CardTitle className="text-base">{t('callsOverTime', 'Calls over time')}</CardTitle>
							<CardDescription>
								{t('modelCallsAndFailuresBucketedBy', 'Model calls and failures bucketed by')}{" "}
								<span className="font-mono">{bucket}</span> {t('overTheSelectedWindow', "over the selected window.")}
							</CardDescription>
						</CardHeader>
						<CardContent>
							<LlmTrendChart points={llm.data?.trend ?? []} bucket={bucket} />
						</CardContent>
					</Card>

					<Card>
						<CardHeader className="pb-3">
							<CardTitle className="text-base">{t('byModel', 'By model')}</CardTitle>
							<CardDescription>
								{t('busiestModelsFirstWithLatencyAndTokenTotalsPerModel', 'Busiest models first, with latency and token totals per model.')}
							</CardDescription>
						</CardHeader>
						<CardContent className="p-0">
							<ModelTable rows={llm.data?.byModel ?? []} />
						</CardContent>
					</Card>

					<div className="grid gap-4 lg:grid-cols-3">
						<Card>
							<CardHeader className="pb-3">
								<CardTitle className="text-base">{t('byProvider', 'By provider')}</CardTitle>
								<CardDescription>
									{t('callVolumeAndErrorRatePerProvider', 'Call volume and error rate per provider.')}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<RateBarList
									rows={providerRows}
									emptyMessage="No provider activity in this window."
								/>
							</CardContent>
						</Card>
						<Card>
							<CardHeader className="pb-3">
								<CardTitle className="text-base">{t('byOperation', 'By operation')}</CardTitle>
								<CardDescription>
									{t('chatEmbeddingAndToolCalls', 'Chat, embedding and tool calls.')}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<RateBarList
									rows={operationRows}
									emptyMessage="No operations in this window."
								/>
							</CardContent>
						</Card>
						<Card>
							<CardHeader className="pb-3">
								<CardTitle className="text-base">{t('topErrorKinds', 'Top error kinds')}</CardTitle>
								<CardDescription>
									{t('classifiedFailureReasonsAcrossAllProviders', 'Classified failure reasons across all providers.')}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<BarList
									rows={errorRows}
									emptyMessage="No failed calls in this window."
								/>
							</CardContent>
						</Card>
					</div>
				</>
			)}

			<AgentBackendsCard profile={profile} hours={hours} />
		</section>
	);
}

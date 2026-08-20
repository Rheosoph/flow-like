"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	Bot,
	CircleCheck,
	CircleX,
	MonitorSmartphone,
	Play,
} from "lucide-react";
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
} from "../../../ui";
import { TelemetryGranularityNotice } from "./granularity-notice";
import {
	BarList,
	EmptyState,
	StatTile,
	type TelemetryBucket,
	formatBucketTick,
	trendBucketForHours,
} from "./telemetry-shared";
import type {
	ITelemetryFlowpilotResponse,
	ITelemetryFlowpilotTotals,
	ITelemetryFlowpilotTrendPoint,
} from "./types";

const runsChartConfig = {
	runsStarted: {
		label: "Started",
		color: "var(--chart-1)",
	},
	runsSucceeded: {
		label: "Succeeded",
		color: "var(--chart-2)",
	},
	runsFailed: {
		label: "Failed",
		color: "var(--destructive)",
	},
} satisfies ChartConfig;

const FUNNEL_STAGES: {
	key: keyof ITelemetryFlowpilotTotals;
	label: string;
}[] = [
	{ key: "attemptsTotal", label: "Attempts" },
	{ key: "attemptsParseValid", label: "Parse valid" },
	{ key: "attemptsTypedValid", label: "Typed valid" },
	{ key: "attemptsReconcileValid", label: "Reconcile valid" },
	{ key: "attemptsApplied", label: "Applied" },
];

function GenerationFunnel({ totals }: { totals: ITelemetryFlowpilotTotals }) {
	const base = totals.attemptsTotal;
	if (base === 0) {
		return <EmptyState message="No generation attempts in this window." />;
	}
	return (
		<div className="space-y-1.5">
			{FUNNEL_STAGES.map((stage) => {
				const count = totals[stage.key];
				const pct = (count / base) * 100;
				return (
					<div key={stage.key} className="flex items-center gap-2 px-1 py-0.5">
						<span className="w-32 truncate text-xs font-medium">
							{stage.label}
						</span>
						<div className="relative h-3 flex-1 overflow-hidden rounded bg-muted">
							<div
								className="h-full rounded bg-primary/60"
								style={{ width: `${Math.min(100, Math.max(0, pct))}%` }}
							/>
						</div>
						<span className="w-16 text-right text-[11px] tabular-nums text-muted-foreground">
							{count.toLocaleString()}
						</span>
						<span className="w-12 text-right text-[11px] tabular-nums text-muted-foreground">
							{pct.toFixed(0)}%
						</span>
					</div>
				);
			})}
		</div>
	);
}

function RunsTrendChart({
	points,
	bucket,
}: {
	points: ITelemetryFlowpilotTrendPoint[];
	bucket: TelemetryBucket;
}) {
	if (points.length === 0) {
		return (
			<EmptyState
				message="No runs in the selected window."
				className="h-64 text-sm"
			/>
		);
	}
	return (
		<ChartContainer config={runsChartConfig} className="h-64 w-full">
			<AreaChart
				data={points}
				margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
			>
				<defs>
					<linearGradient id="flowpilotRunsStarted" x1="0" y1="0" x2="0" y2="1">
						<stop
							offset="0%"
							stopColor="var(--color-runsStarted)"
							stopOpacity={0.2}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-runsStarted)"
							stopOpacity={0.02}
						/>
					</linearGradient>
					<linearGradient
						id="flowpilotRunsSucceeded"
						x1="0"
						y1="0"
						x2="0"
						y2="1"
					>
						<stop
							offset="0%"
							stopColor="var(--color-runsSucceeded)"
							stopOpacity={0.3}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-runsSucceeded)"
							stopOpacity={0.03}
						/>
					</linearGradient>
					<linearGradient id="flowpilotRunsFailed" x1="0" y1="0" x2="0" y2="1">
						<stop
							offset="0%"
							stopColor="var(--color-runsFailed)"
							stopOpacity={0.3}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-runsFailed)"
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
					width={40}
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
					dataKey="runsStarted"
					stroke="var(--color-runsStarted)"
					strokeDasharray="6 3"
					fill="url(#flowpilotRunsStarted)"
					strokeWidth={2}
				/>
				<Area
					type="monotone"
					dataKey="runsSucceeded"
					stroke="var(--color-runsSucceeded)"
					fill="url(#flowpilotRunsSucceeded)"
					strokeWidth={2}
				/>
				<Area
					type="monotone"
					dataKey="runsFailed"
					stroke="var(--color-runsFailed)"
					fill="url(#flowpilotRunsFailed)"
					strokeWidth={2}
				/>
			</AreaChart>
		</ChartContainer>
	);
}

interface FlowpilotSectionProps {
	profile: IProfile | undefined;
	hours: number;
}

export function FlowpilotSection({
	profile,
	hours,
}: Readonly<FlowpilotSectionProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const flowpilot = useQuery<ITelemetryFlowpilotResponse>({
		queryKey: ["admin", "telemetry", "flowpilot", hours],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryFlowpilotResponse>(
				profile,
				`admin/telemetry/flowpilot?hours=${hours}`,
			);
		},
		enabled: !!profile,
	});

	const totals = flowpilot.data?.totals;
	const bucket = trendBucketForHours(flowpilot.data?.hours ?? hours);
	const successRate =
		totals && totals.runsStarted > 0
			? (totals.runsSucceeded / totals.runsStarted) * 100
			: null;

	return (
		<section className="space-y-4">
			<h2 className="flex items-center gap-2 text-xl font-semibold">
				<Bot className="h-5 w-5 text-primary" />
				{t("flowpilot", "FlowPilot")}
				<TelemetryGranularityNotice response={flowpilot.data} />
			</h2>

			{flowpilot.isLoading ? (
				<div className="space-y-4">
					<div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
						{["runs", "success", "cancelled", "installs"].map((k) => (
							<Skeleton key={k} className="h-16" />
						))}
					</div>
					<Skeleton className="h-64 w-full" />
				</div>
			) : !totals || totals.runsStarted === 0 ? (
				<EmptyState
					message="No FlowPilot telemetry in this window."
					className="py-10 text-sm"
				/>
			) : (
				<>
					<div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
						<StatTile
							label="Runs"
							value={totals.runsStarted.toLocaleString()}
							icon={<Play className="h-4 w-4" />}
							hint={t("valFailed", "{{val}} failed", {
								val: totals.runsFailed.toLocaleString(),
							})}
						/>
						<StatTile
							label={t("successRate", "Success rate")}
							value={successRate == null ? "—" : `${successRate.toFixed(1)}%`}
							icon={<CircleCheck className="h-4 w-4" />}
							hint={t("valOfVal2Runs", "{{val}} of {{val2}} runs", {
								val: totals.runsSucceeded.toLocaleString(),
								val2: totals.runsStarted.toLocaleString(),
							})}
						/>
						<StatTile
							label={t("cancelled", "Cancelled")}
							value={totals.runsCancelled.toLocaleString()}
							icon={<CircleX className="h-4 w-4" />}
							hint="Stopped by the user"
						/>
						<StatTile
							label={t("installsReporting", "Installs reporting")}
							value={(flowpilot.data?.installs ?? 0).toLocaleString()}
							icon={<MonitorSmartphone className="h-4 w-4" />}
							hint="Distinct anonymous ids"
						/>
					</div>

					<Card>
						<CardHeader className="pb-3">
							<CardTitle className="text-base">
								{t("runsOverTime", "Runs over time")}
							</CardTitle>
							<CardDescription>
								{t("generationRunsBucketedBy", "Generation runs bucketed by")}{" "}
								<span className="font-mono">{bucket}</span>{" "}
								{t("overTheSelectedWindow", "over the selected window.")}
							</CardDescription>
						</CardHeader>
						<CardContent>
							<RunsTrendChart
								points={flowpilot.data?.trend ?? []}
								bucket={bucket}
							/>
						</CardContent>
					</Card>

					<div className="grid gap-4 lg:grid-cols-2">
						<Card>
							<CardHeader className="pb-3">
								<CardTitle className="text-base">
									{t("generationFunnel", "Generation funnel")}
								</CardTitle>
								<CardDescription>
									{t(
										"attemptsSurvivingEachValidationStageAsAShareOfAllAttempts",
										"Attempts surviving each validation stage, as a share of all attempts.",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<GenerationFunnel totals={totals} />
							</CardContent>
						</Card>
						<Card>
							<CardHeader className="pb-3">
								<CardTitle className="text-base">
									{t("reviewDispositions", "Review dispositions")}
								</CardTitle>
								<CardDescription>
									{totals.queuedReviews.toLocaleString()}{" "}
									{t(
										"reviewsQueuedInThisWindow",
										"reviews queued in this window.",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<BarList
									rows={[
										{
											key: "applied",
											label: "applied",
											count: totals.applyDispositions,
										},
										{
											key: "dismissed",
											label: "dismissed",
											count: totals.dismissedDispositions,
										},
										{
											key: "stale",
											label: "stale",
											count: totals.staleDispositions,
										},
										{
											key: "error",
											label: "error",
											count: totals.errorDispositions,
										},
									]}
									emptyMessage="No review dispositions in this window."
								/>
							</CardContent>
						</Card>
					</div>

					<Card>
						<CardHeader className="pb-3">
							<CardTitle className="text-base">
								{t("quality", "Quality")}
							</CardTitle>
							<CardDescription>
								{t(
									"diagnosticAndValidationSignalsAcrossAllReportedRuns",
									"Diagnostic and validation signals across all reported runs.",
								)}
							</CardDescription>
						</CardHeader>
						<CardContent>
							<div className="grid gap-2 sm:grid-cols-3 lg:grid-cols-6">
								<StatTile
									label="Diagnostics"
									value={totals.diagnosticOccurrences.toLocaleString()}
								/>
								<StatTile
									label={t("repeatedDiagnostics", "Repeated diagnostics")}
									value={totals.repeatedDiagnosticOccurrences.toLocaleString()}
								/>
								<StatTile
									label={t("validationRegressions", "Validation regressions")}
									value={totals.validationRegressions.toLocaleString()}
								/>
								<StatTile
									label={t("emptyBoardsAfterRun", "Empty boards after run")}
									value={totals.emptyBoardsAfterRun.toLocaleString()}
								/>
								<StatTile
									label={t("boardsInspected", "Boards inspected")}
									value={totals.boardsInspected.toLocaleString()}
								/>
								<StatTile
									label={t("plansFeasible", "Plans feasible")}
									value={`${totals.plansFeasible.toLocaleString()} / ${totals.plansInfeasible.toLocaleString()}`}
									hint={t(
										"feasibleInfeasibleOfValAssessed",
										"feasible / infeasible of {{val}} assessed",
										{ val: totals.plansAssessed.toLocaleString() },
									)}
								/>
							</div>
						</CardContent>
					</Card>
				</>
			)}
		</section>
	);
}

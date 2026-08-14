"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	CalendarRange,
	Repeat,
	UserMinus,
	UserPlus,
	Users,
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
	ChartTooltip,
	ChartTooltipContent,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
} from "../../../ui";
import { TelemetryGranularityNotice } from "./granularity-notice";
import {
	BarList,
	EmptyState,
	StatTile,
	formatBucketTick,
} from "./telemetry-shared";
import type {
	ITelemetryDauPoint,
	ITelemetryEngagementResponse,
	ITelemetryRetentionCohort,
} from "./types";

const DAY_OPTIONS: { value: number; label: string }[] = [
	{ value: 30, label: "Last 30 days" },
	{ value: 60, label: "Last 60 days" },
	{ value: 90, label: "Last 90 days" },
];

const WEEK_OFFSETS = [0, 1, 2, 3, 4, 5, 6, 7] as const;

const dauChartConfig = {
	installs: {
		label: "Daily active installs",
		color: "var(--chart-2)",
	},
} satisfies ChartConfig;

function DauChart({ points }: { points: ITelemetryDauPoint[] }) {
	if (points.length === 0) {
		return (
			<EmptyState
				message="No engagement data in the selected window."
				className="h-64 text-sm"
			/>
		);
	}
	return (
		<ChartContainer config={dauChartConfig} className="h-64 w-full">
			<AreaChart
				data={points}
				margin={{ top: 8, right: 8, left: 0, bottom: 0 }}
			>
				<defs>
					<linearGradient id="engagementDauFill" x1="0" y1="0" x2="0" y2="1">
						<stop
							offset="0%"
							stopColor="var(--color-installs)"
							stopOpacity={0.3}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-installs)"
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
					tickFormatter={(v) => formatBucketTick(v as string, "day")}
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
								formatBucketTick(value as string, "day")
							}
						/>
					}
				/>
				<Area
					type="monotone"
					dataKey="installs"
					stroke="var(--color-installs)"
					fill="url(#engagementDauFill)"
					strokeWidth={2}
				/>
			</AreaChart>
		</ChartContainer>
	);
}

function formatCohortWeek(value: string) {
	const d = new Date(value);
	if (Number.isNaN(d.getTime())) return value;
	return d.toLocaleDateString([], {
		month: "short",
		day: "numeric",
		timeZone: "UTC",
	});
}

function RetentionGrid({
	cohorts,
}: {
	cohorts: ITelemetryRetentionCohort[];
}) {
	const { t } = useTranslation("admin");
	if (cohorts.length === 0) {
		return (
			<EmptyState message="Not enough history for retention cohorts yet." />
		);
	}
	return (
		<div className="overflow-x-auto">
			<table className="w-full min-w-[36rem] border-separate border-spacing-0.5">
				<thead>
					<tr>
						<th className="px-2 py-1 text-left text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
							{t('cohort', 'Cohort')}
						</th>
						<th className="px-2 py-1 text-right text-[10px] font-medium uppercase tracking-wide text-muted-foreground">
							{t('size', 'Size')}
						</th>
						{WEEK_OFFSETS.map((w) => (
							<th
								key={w}
								className="px-1 py-1 text-center text-[10px] font-medium uppercase tracking-wide text-muted-foreground"
							>{`W${w}`}</th>
						))}
					</tr>
				</thead>
				<tbody>
					{cohorts.map((c) => (
						<tr key={c.cohortWeek}>
							<td className="whitespace-nowrap px-2 py-1 text-xs tabular-nums text-muted-foreground">
								{formatCohortWeek(c.cohortWeek)}
							</td>
							<td className="px-2 py-1 text-right text-xs font-medium tabular-nums">
								{c.cohortSize.toLocaleString()}
							</td>
							{WEEK_OFFSETS.map((w) => {
								const ratio = c.weeks[w];
								if (ratio == null) {
									return <td key={w} />;
								}
								const clamped = Math.min(1, Math.max(0, ratio));
								return (
									<td key={w} className="p-0">
										<div className="relative min-w-11 overflow-hidden rounded">
											<div
												className="absolute inset-0 bg-primary"
												style={{ opacity: clamped * 0.75 }}
											/>
											<div className="relative px-1 py-1.5 text-center text-[11px] tabular-nums text-foreground">
												{Math.round(clamped * 100)}%
											</div>
										</div>
									</td>
								);
							})}
						</tr>
					))}
				</tbody>
			</table>
		</div>
	);
}

interface EngagementSectionProps {
	profile: IProfile | undefined;
	days: number;
	onDaysChange: (days: number) => void;
}

export function EngagementSection({
	profile,
	days,
	onDaysChange,
}: Readonly<EngagementSectionProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const engagement = useQuery<ITelemetryEngagementResponse>({
		queryKey: ["admin", "telemetry", "engagement", days],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryEngagementResponse>(
				profile,
				`admin/telemetry/engagement?days=${days}`,
			);
		},
		enabled: !!profile,
	});

	const data = engagement.data;
	const churnRate = data?.churnRate ?? null;

	return (
		<section className="space-y-4">
			<div className="flex flex-wrap items-center justify-between gap-2">
				<h2 className="flex items-center gap-2 text-xl font-semibold">
					<Users className="h-5 w-5 text-primary" />
					{t('engagement', 'Engagement')}
					<TelemetryGranularityNotice response={data} />
				</h2>
				<Select
					value={String(days)}
					onValueChange={(v) => onDaysChange(Number.parseInt(v, 10))}
				>
					<SelectTrigger className="w-36">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{DAY_OPTIONS.map((o) => (
							<SelectItem key={o.value} value={String(o.value)}>
								{o.label}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>

			{engagement.isLoading ? (
				<div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-5">
					{["wau", "mau", "new", "returning", "churned"].map((k) => (
						<Skeleton key={k} className="h-16" />
					))}
				</div>
			) : (
				<div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-5">
					<StatTile
						label="WAU"
						value={(data?.wau ?? 0).toLocaleString()}
						icon={<Users className="h-4 w-4" />}
						hint={t('prevVal', 'prev {{val}}', { val: (data?.previousWau ?? 0).toLocaleString() })}
					/>
					<StatTile
						label="MAU"
						value={(data?.mau ?? 0).toLocaleString()}
						icon={<CalendarRange className="h-4 w-4" />}
						hint={t('prevVal', 'prev {{val}}', { val: (data?.previousMau ?? 0).toLocaleString() })}
					/>
					<StatTile
						label={t('newInstalls', 'New installs')}
						value={(data?.newInstalls ?? 0).toLocaleString()}
						icon={<UserPlus className="h-4 w-4" />}
						hint="First seen in window"
					/>
					<StatTile
						label="Returning"
						value={(data?.returningInstalls ?? 0).toLocaleString()}
						icon={<Repeat className="h-4 w-4" />}
						hint="Active, not new"
					/>
					<StatTile
						label="Churned"
						value={(data?.churnedInstalls ?? 0).toLocaleString()}
						icon={<UserMinus className="h-4 w-4" />}
						extra={
							churnRate != null && churnRate > 0 ? (
								<span className="inline-flex items-center rounded-full border border-destructive/30 bg-destructive/10 px-1.5 py-0.5 text-[10px] font-medium tabular-nums text-destructive">
									{(churnRate * 100).toFixed(0)}{t('churn', '% churn')}
								</span>
							) : undefined
						}
						hint="Active before, silent now"
					/>
				</div>
			)}

			<Card>
				<CardHeader className="pb-3">
					<CardTitle className="text-base">{t('dailyActiveInstalls', 'Daily active installs')}</CardTitle>
					<CardDescription>
						{t('distinctInstallsPerUtcDayOverTheSelectedWindow', 'Distinct installs per UTC day over the selected window.')}
					</CardDescription>
				</CardHeader>
				<CardContent>
					{engagement.isLoading ? (
						<Skeleton className="h-64 w-full" />
					) : (
						<DauChart points={data?.dau ?? []} />
					)}
				</CardContent>
			</Card>

			<div className="grid gap-4 lg:grid-cols-3">
				<Card className="lg:col-span-2">
					<CardHeader className="pb-3">
						<CardTitle className="text-base">{t('weeklyRetention', 'Weekly retention')}</CardTitle>
						<CardDescription>
							{t('shareOfEachFirstseenCohortStillActiveNWeeksLater', 'Share of each first-seen cohort still active N weeks later.')}
						</CardDescription>
					</CardHeader>
					<CardContent>
						{engagement.isLoading ? (
							<div className="space-y-1.5">
								<Skeleton className="h-6 w-full" />
								<Skeleton className="h-6 w-full" />
								<Skeleton className="h-6 w-full" />
							</div>
						) : (
							<RetentionGrid cohorts={data?.retention ?? []} />
						)}
					</CardContent>
				</Card>
				<Card>
					<CardHeader className="pb-3">
						<CardTitle className="text-base">
							{t('lastScreenBeforeDropoff', 'Last screen before drop-off')}
						</CardTitle>
						<CardDescription>
							{t('finalPageSeenByInstallsThatStoppedComingBack', 'Final page seen by installs that stopped coming back.')}
						</CardDescription>
					</CardHeader>
					<CardContent>
						<BarList
							rows={
								data?.dropOffPaths.map((p) => ({
									key: p.path,
									label: p.path,
									count: p.count,
								})) ?? []
							}
							loading={engagement.isLoading}
							emptyMessage="No churned installs in this window."
						/>
					</CardContent>
				</Card>
			</div>
		</section>
	);
}

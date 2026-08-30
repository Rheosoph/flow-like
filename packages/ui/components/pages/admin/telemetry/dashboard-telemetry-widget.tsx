"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	Activity,
	ArrowDownRight,
	ArrowRight,
	ArrowUpRight,
	ExternalLink,
	Layers,
	MonitorSmartphone,
	Star,
	TrendingUp,
	Zap,
} from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Skeleton,
} from "../../../ui";
import type {
	ITelemetryOverviewResponse,
	ITelemetryTimeseriesResponse,
} from "./types";

interface DashboardTelemetryWidgetProps {
	profile: IProfile | undefined;
}

function StatTile({
	label,
	value,
	icon,
}: {
	label: string;
	value: string;
	icon: React.ReactNode;
}) {
	return (
		<div className="flex items-center gap-2 rounded-lg border border-border bg-muted/40 px-3 py-2">
			<div className="text-muted-foreground">{icon}</div>
			<div className="min-w-0">
				<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
					{label}
				</div>
				<div className="truncate text-sm font-semibold tabular-nums">
					{value}
				</div>
			</div>
		</div>
	);
}

function Sparkline({
	points,
	height = 36,
}: {
	points: { count: number }[];
	height?: number;
}) {
	const { t } = useTranslation("admin");
	const path = useMemo(() => {
		if (!points.length) return "";
		const max = Math.max(1, ...points.map((p) => p.count));
		const step = 100 / Math.max(1, points.length - 1);
		return points
			.map((p, i) => {
				const x = i * step;
				const y = 100 - (p.count / max) * 100;
				return `${i === 0 ? "M" : "L"} ${x.toFixed(2)} ${y.toFixed(2)}`;
			})
			.join(" ");
	}, [points]);

	if (!points.length) {
		return (
			<div className="flex h-9 items-center text-xs text-muted-foreground">
				{t("noData", "No data")}
			</div>
		);
	}

	return (
		<svg
			role="presentation"
			viewBox="0 0 100 100"
			preserveAspectRatio="none"
			className="w-full"
			style={{ height }}
		>
			<defs>
				<linearGradient id="telemetrySparkFill" x1="0" y1="0" x2="0" y2="1">
					<stop offset="0%" stopColor="var(--chart-1)" stopOpacity="0.4" />
					<stop offset="100%" stopColor="var(--chart-1)" stopOpacity="0" />
				</linearGradient>
			</defs>
			<path d={`${path} L 100 100 L 0 100 Z`} fill="url(#telemetrySparkFill)" />
			<path
				d={path}
				fill="none"
				stroke="var(--chart-1)"
				strokeWidth="1.5"
				vectorEffect="non-scaling-stroke"
				strokeLinecap="round"
				strokeLinejoin="round"
			/>
		</svg>
	);
}

function TopList({
	title,
	buckets,
	loading,
	hrefBuilder,
}: {
	title: string;
	buckets: { key: string; label: string; count: number }[];
	loading: boolean;
	hrefBuilder: (b: { key: string; label: string; count: number }) => string;
}) {
	const { t } = useTranslation("admin");
	const max = Math.max(1, ...buckets.map((b) => b.count));
	return (
		<div className="rounded-lg border bg-card/50 p-3">
			<div className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
				{title}
			</div>
			{loading ? (
				<div className="space-y-1.5">
					<Skeleton className="h-4 w-full" />
					<Skeleton className="h-4 w-full" />
					<Skeleton className="h-4 w-full" />
				</div>
			) : buckets.length === 0 ? (
				<div className="text-xs text-muted-foreground">
					{t("noData", "No data")}
				</div>
			) : (
				<ul className="space-y-1">
					{buckets.map((b) => (
						<li key={b.key}>
							<Link
								href={hrefBuilder(b)}
								className="group flex items-center gap-2 rounded px-1 py-0.5 hover:bg-muted/50"
							>
								<span className="w-32 truncate font-mono text-xs font-medium group-hover:text-primary">
									{b.label}
								</span>
								<div className="relative flex-1 overflow-hidden rounded-full bg-muted h-1.5">
									<div
										className="h-full rounded-full bg-primary/60"
										style={{ width: `${(b.count / max) * 100}%` }}
									/>
								</div>
								<span className="w-10 text-right text-[11px] tabular-nums text-muted-foreground">
									{b.count.toLocaleString()}
								</span>
							</Link>
						</li>
					))}
				</ul>
			)}
		</div>
	);
}

export function DashboardTelemetryWidget({
	profile,
}: DashboardTelemetryWidgetProps) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const overview = useQuery<ITelemetryOverviewResponse>({
		queryKey: ["admin", "telemetry", "overview", "dashboard"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryOverviewResponse>(
				profile,
				"admin/telemetry/overview?hours=24",
			);
		},
		enabled: !!profile,
		refetchInterval: 60_000,
	});

	const series = useQuery<ITelemetryTimeseriesResponse>({
		queryKey: ["admin", "telemetry", "timeseries", "dashboard"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryTimeseriesResponse>(
				profile,
				"admin/telemetry/timeseries?hours=24&bucket=hour",
			);
		},
		enabled: !!profile,
		refetchInterval: 60_000,
	});

	const change = useMemo(() => {
		if (!overview.data) return null;
		const { totalEvents, previousTotalEvents } = overview.data;
		if (previousTotalEvents <= 0) return null;
		return ((totalEvents - previousTotalEvents) / previousTotalEvents) * 100;
	}, [overview.data]);

	const ChangeIcon =
		change == null
			? ArrowRight
			: change > 0
				? ArrowUpRight
				: change < 0
					? ArrowDownRight
					: ArrowRight;

	const topEvent = overview.data?.topEvents[0];
	const isEmpty =
		!overview.isLoading && (overview.data?.totalEvents ?? 0) === 0;

	return (
		<Card className="overflow-hidden border-primary/20">
			<CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0 pb-3">
				<div className="space-y-1">
					<CardTitle className="flex items-center gap-2 text-base">
						<Activity className="h-4 w-4 text-primary" />
						{t("telemetry", "Telemetry")}
						<Badge variant="outline" className="text-[10px]">
							24h
						</Badge>
					</CardTitle>
					<CardDescription>
						{t(
							"anonymousOptinProductMetricsAcrossInstalls",
							"Anonymous opt-in product metrics across installs",
						)}
					</CardDescription>
				</div>
				<Button asChild size="sm" variant="outline">
					<Link href="/admin/telemetry">
						{t("openTelemetry", "Open Telemetry")}
						<ExternalLink className="ml-1 h-3 w-3" />
					</Link>
				</Button>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid gap-2 sm:grid-cols-4">
					<StatTile
						label={t("events24h", "Events 24h")}
						value={
							overview.isLoading
								? "…"
								: (overview.data?.totalEvents ?? 0).toLocaleString()
						}
						icon={<Zap className="h-4 w-4" />}
					/>
					<StatTile
						label={t("activeInstalls", "Active installs")}
						value={
							overview.isLoading
								? "…"
								: (overview.data?.activeInstalls ?? 0).toLocaleString()
						}
						icon={<MonitorSmartphone className="h-4 w-4" />}
					/>
					<StatTile
						label={t("topEvent", "Top event")}
						value={overview.isLoading ? "…" : (topEvent?.name ?? "—")}
						icon={<Star className="h-4 w-4" />}
					/>
					<StatTile
						label="Sources"
						value={
							overview.isLoading
								? "…"
								: String(overview.data?.sources.length ?? 0)
						}
						icon={<Layers className="h-4 w-4" />}
					/>
				</div>

				{overview.isLoading ? (
					<div className="space-y-2">
						<Skeleton className="h-16 w-full" />
						<Skeleton className="h-8 w-full" />
					</div>
				) : isEmpty ? (
					<div className="flex items-center justify-center rounded-lg border border-dashed py-8 text-sm text-muted-foreground">
						{t(
							"noTelemetryYetDataAppearsOnceUsersOptIn",
							"No telemetry yet — data appears once users opt in.",
						)}
					</div>
				) : (
					<>
						<div className="rounded-lg border bg-card/50 p-3">
							<div className="mb-2 flex items-center justify-between text-xs text-muted-foreground">
								<span className="inline-flex items-center gap-1.5">
									<TrendingUp className="h-3 w-3" />{" "}
									{t("eventsLast24hHourly", "Events (last 24h, hourly)")}
								</span>
								{change != null && (
									<span
										className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] tabular-nums ${
											change > 5
												? "border-emerald-500/40 text-emerald-600 dark:text-emerald-400"
												: "border-border text-muted-foreground"
										}`}
									>
										<ChangeIcon className="h-3 w-3" />
										{change >= 0 ? "+" : ""}
										{change.toFixed(1)}
										{t("vsPrior", "% vs prior")}
									</span>
								)}
							</div>
							<Sparkline points={series.data?.points ?? []} />
						</div>

						<TopList
							title={t("topEvents", "Top events")}
							buckets={
								overview.data?.topEvents.map((e) => ({
									key: e.name,
									label: e.name,
									count: e.count,
								})) ?? []
							}
							loading={overview.isLoading}
							hrefBuilder={(b) =>
								`/admin/telemetry?name=${encodeURIComponent(b.key)}`
							}
						/>
					</>
				)}
			</CardContent>
		</Card>
	);
}

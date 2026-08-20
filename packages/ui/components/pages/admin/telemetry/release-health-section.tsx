"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	Activity,
	HeartPulse,
	MonitorSmartphone,
	ServerCrash,
	ShieldCheck,
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
	ChartTooltip,
	ChartTooltipContent,
	RelativeTime,
	Skeleton,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../../../ui";
import { TelemetryGranularityNotice } from "./granularity-notice";
import {
	EmptyState,
	type TelemetryBucket,
	formatBucketTick,
	trendBucketForHours,
} from "./telemetry-shared";
import type {
	ITelemetryReleaseHealthPoint,
	ITelemetryReleaseHealthResponse,
	ITelemetryReleaseRow,
} from "./types";

const crashFreeChartConfig = {
	crashFreeRate: {
		label: "Crash-free sessions",
		color: "var(--chart-2)",
	},
} satisfies ChartConfig;

export function formatRatePercent(
	rate: number | null | undefined,
	digits = 2,
): string {
	if (rate == null || Number.isNaN(rate)) return "—";
	return `${(rate * 100).toFixed(digits)}%`;
}

function HeroTile({
	label,
	value,
	icon,
	hint,
	emphasis,
}: {
	readonly label: string;
	readonly value: string;
	readonly icon: React.ReactNode;
	readonly hint?: string;
	readonly emphasis?: boolean;
}) {
	return (
		<div
			className={`rounded-xl border p-4 ${emphasis ? "border-emerald-500/30 bg-emerald-500/5" : "border-border bg-muted/40"}`}
		>
			<div className="flex items-center justify-between text-muted-foreground">
				<span className="text-xs uppercase tracking-wide">{label}</span>
				{icon}
			</div>
			<div
				className={`mt-1 truncate tabular-nums font-bold ${emphasis ? "text-4xl" : "text-2xl"}`}
			>
				{value}
			</div>
			{hint ? (
				<div className="mt-1 truncate text-xs text-muted-foreground">
					{hint}
				</div>
			) : null}
		</div>
	);
}

function CrashFreeTrendChart({
	points,
	bucket,
}: {
	readonly points: ITelemetryReleaseHealthPoint[];
	readonly bucket: TelemetryBucket;
}) {
	const { t } = useTranslation("admin");
	const data = useMemo(
		() =>
			points.map((p) => ({
				ts: p.ts,
				crashFreeRate:
					p.crashFreeSessionRate == null ? null : p.crashFreeSessionRate * 100,
			})),
		[points],
	);

	const lowerBound = useMemo(() => {
		const values = data
			.map((p) => p.crashFreeRate)
			.filter((v): v is number => v != null);
		if (values.length === 0) return 0;
		return Math.max(0, Math.floor(Math.min(...values) - 1));
	}, [data]);

	if (data.length === 0) {
		return (
			<EmptyState
				message="No sessions reported in the selected window."
				className="h-64 text-sm"
			/>
		);
	}

	return (
		<ChartContainer config={crashFreeChartConfig} className="h-64 w-full">
			<AreaChart data={data} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
				<defs>
					<linearGradient
						id="releaseHealthCrashFree"
						x1="0"
						y1="0"
						x2="0"
						y2="1"
					>
						<stop
							offset="0%"
							stopColor="var(--color-crashFreeRate)"
							stopOpacity={0.3}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-crashFreeRate)"
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
					domain={[lowerBound, 100]}
					tickLine={false}
					axisLine={false}
					tick={{ fontSize: 11 }}
					tickFormatter={(v) => `${Number(v).toFixed(0)}%`}
					width={48}
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
									{t("crashfreeSessions", "Crash-free sessions")}{" "}
									<span className="font-medium tabular-nums text-foreground">
										{Number(value).toFixed(2)}%
									</span>
								</span>
							)}
						/>
					}
				/>
				<Area
					type="monotone"
					dataKey="crashFreeRate"
					stroke="var(--color-crashFreeRate)"
					fill="url(#releaseHealthCrashFree)"
					strokeWidth={2}
					connectNulls
				/>
			</AreaChart>
		</ChartContainer>
	);
}

function AdoptionBar({ adoption }: { readonly adoption?: number | null }) {
	if (adoption == null) {
		return <span className="text-xs text-muted-foreground">—</span>;
	}
	const pct = Math.min(100, Math.max(0, adoption * 100));
	return (
		<div className="flex items-center gap-2">
			<div className="relative h-2 w-24 overflow-hidden rounded-full bg-muted">
				<div
					className="h-full rounded-full bg-primary/60"
					style={{ width: `${pct}%` }}
				/>
			</div>
			<span className="w-10 text-right text-[11px] tabular-nums text-muted-foreground">
				{pct.toFixed(0)}%
			</span>
		</div>
	);
}

function ReleasesTable({
	releases,
}: { readonly releases: ITelemetryReleaseRow[] }) {
	const { t } = useTranslation("admin");
	const sorted = useMemo(
		() =>
			[...releases].sort(
				(a, b) =>
					new Date(b.firstSeenAt).getTime() - new Date(a.firstSeenAt).getTime(),
			),
		[releases],
	);

	if (sorted.length === 0) {
		return (
			<EmptyState
				message="No releases reported in the selected window."
				className="m-4 py-10 text-sm"
			/>
		);
	}

	return (
		<Table>
			<TableHeader>
				<TableRow>
					<TableHead>{t("version", "Version")}</TableHead>
					<TableHead>{t("adoption", "Adoption")}</TableHead>
					<TableHead className="text-right">
						{t("installs", "Installs")}
					</TableHead>
					<TableHead className="text-right">
						{t("sessions", "Sessions")}
					</TableHead>
					<TableHead className="text-right">
						{t("crashfree", "Crash-free")}
					</TableHead>
					<TableHead className="text-right">{t("errors", "Errors")}</TableHead>
				</TableRow>
			</TableHeader>
			<TableBody>
				{sorted.map((release) => (
					<TableRow key={`${release.source}:${release.version}`}>
						<TableCell>
							<div className="font-mono text-xs font-medium">
								{release.version}
							</div>
							<div className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
								<span className="font-mono">{release.source}</span>
								<span>·</span>
								<RelativeTime value={release.firstSeenAt} />
							</div>
						</TableCell>
						<TableCell>
							<AdoptionBar adoption={release.adoption} />
						</TableCell>
						<TableCell className="text-right text-xs tabular-nums">
							{release.installs.toLocaleString()}
						</TableCell>
						<TableCell className="text-right text-xs tabular-nums">
							{release.sessions.toLocaleString()}
						</TableCell>
						<TableCell className="text-right text-xs tabular-nums">
							{formatRatePercent(release.crashFreeSessionRate)}
						</TableCell>
						<TableCell className="text-right text-xs tabular-nums text-muted-foreground">
							{release.errorCount.toLocaleString()}
						</TableCell>
					</TableRow>
				))}
			</TableBody>
		</Table>
	);
}

interface ReleaseHealthSectionProps {
	profile: IProfile | undefined;
	hours: number;
	source?: string;
}

export function ReleaseHealthSection({
	profile,
	hours,
	source,
}: Readonly<ReleaseHealthSectionProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const health = useQuery<ITelemetryReleaseHealthResponse>({
		queryKey: ["admin", "telemetry", "release-health", hours, source ?? "all"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			const params = new URLSearchParams({ hours: String(hours) });
			if (source && source !== "all") params.set("source", source);
			return backend.apiState.get<ITelemetryReleaseHealthResponse>(
				profile,
				`admin/telemetry/release-health?${params.toString()}`,
			);
		},
		enabled: !!profile,
	});

	const bucket = trendBucketForHours(health.data?.hours ?? hours);
	const hasData =
		(health.data?.totalSessions ?? 0) > 0 ||
		(health.data?.releases.length ?? 0) > 0;

	return (
		<section className="space-y-4">
			<h2 className="flex items-center gap-2 text-xl font-semibold">
				<HeartPulse className="h-5 w-5 text-primary" />
				{t("releaseHealth", "Release health")}
				<TelemetryGranularityNotice response={health.data} />
			</h2>

			{health.isLoading ? (
				<div className="space-y-4">
					<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
						{[
							"crash-free-sessions",
							"crash-free-installs",
							"sessions",
							"crashed",
						].map((k) => (
							<Skeleton key={k} className="h-28" />
						))}
					</div>
					<Skeleton className="h-64 w-full" />
					<Skeleton className="h-40 w-full" />
				</div>
			) : !hasData ? (
				<EmptyState
					message="No session telemetry in this window — release health appears once installs report sessions."
					className="py-10 text-sm"
				/>
			) : (
				<>
					<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
						<HeroTile
							label={t("crashfreeSessions", "Crash-free sessions")}
							value={formatRatePercent(health.data?.crashFreeSessionRate)}
							icon={<ShieldCheck className="h-4 w-4" />}
							hint={t("valSessions", "{{val}} sessions", {
								val: (health.data?.totalSessions ?? 0).toLocaleString(),
							})}
							emphasis
						/>
						<HeroTile
							label={t("crashfreeInstalls", "Crash-free installs")}
							value={formatRatePercent(health.data?.crashFreeInstallRate)}
							icon={<MonitorSmartphone className="h-4 w-4" />}
							hint={t("valInstalls", "{{val}} installs", {
								val: (health.data?.totalInstalls ?? 0).toLocaleString(),
							})}
							emphasis
						/>
						<HeroTile
							label={t("totalSessions", "Total sessions")}
							value={(health.data?.totalSessions ?? 0).toLocaleString()}
							icon={<Activity className="h-4 w-4" />}
							hint="Reported in the selected window"
						/>
						<HeroTile
							label={t("crashedSessions", "Crashed sessions")}
							value={(health.data?.crashedSessions ?? 0).toLocaleString()}
							icon={<ServerCrash className="h-4 w-4" />}
							hint="Sessions ending in a crash"
						/>
					</div>

					<Card>
						<CardHeader className="pb-3">
							<CardTitle className="text-base">
								{t("crashfreeRateOverTime", "Crash-free rate over time")}
							</CardTitle>
							<CardDescription>
								{t(
									"shareOfSessionsWithoutACrashBucketedBy",
									"Share of sessions without a crash, bucketed by",
								)}{" "}
								<span className="font-mono">{bucket}</span>{" "}
								{t("overTheSelectedWindow", "over the selected window.")}
							</CardDescription>
						</CardHeader>
						<CardContent>
							<CrashFreeTrendChart
								points={health.data?.trend ?? []}
								bucket={bucket}
							/>
						</CardContent>
					</Card>

					<Card>
						<CardHeader className="pb-3">
							<CardTitle className="text-base">
								{t("releases", "Releases")}
							</CardTitle>
							<CardDescription>
								{t(
									"adoptionAndStabilityPerReleaseNewestFirst",
									"Adoption and stability per release, newest first.",
								)}
							</CardDescription>
						</CardHeader>
						<CardContent className="p-0">
							<ReleasesTable releases={health.data?.releases ?? []} />
						</CardContent>
					</Card>
				</>
			)}
		</section>
	);
}

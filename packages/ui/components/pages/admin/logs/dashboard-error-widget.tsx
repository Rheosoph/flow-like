"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	ArrowDownRight,
	ArrowRight,
	ArrowUpRight,
	Bug,
	ExternalLink,
	ServerCrash,
	Skull,
	TrendingUp,
	UsersRound,
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
	RelativeTime,
	Skeleton,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../../../ui";
import type { IErrorStatsResponse, IErrorTimeseriesResponse } from "./types";
import { statusCodeTone } from "./types";
import { UserPill } from "./user-pill";

interface DashboardErrorWidgetProps {
	profile: IProfile | undefined;
}

function StatTile({
	label,
	value,
	icon,
	tone = "muted",
}: {
	label: string;
	value: string;
	icon: React.ReactNode;
	tone?: "muted" | "destructive" | "warn" | "good";
}) {
	const ring =
		tone === "destructive"
			? "border-destructive/30 bg-destructive/5"
			: tone === "warn"
				? "border-amber-500/30 bg-amber-500/5"
				: tone === "good"
					? "border-emerald-500/30 bg-emerald-500/5"
					: "border-border bg-muted/40";
	return (
		<div
			className={`flex items-center gap-2 rounded-lg border ${ring} px-3 py-2`}
		>
			<div className="text-muted-foreground">{icon}</div>
			<div>
				<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
					{label}
				</div>
				<div className="text-sm font-semibold tabular-nums">{value}</div>
			</div>
		</div>
	);
}

function Sparkline({
	points,
	height = 36,
}: {
	points: { total: number }[];
	height?: number;
}) {
	const { t } = useTranslation("admin");
	const path = useMemo(() => {
		if (!points.length) return "";
		const max = Math.max(1, ...points.map((p) => p.total));
		const step = 100 / Math.max(1, points.length - 1);
		return points
			.map((p, i) => {
				const x = i * step;
				const y = 100 - (p.total / max) * 100;
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
				<linearGradient id="errSparkFill" x1="0" y1="0" x2="0" y2="1">
					<stop offset="0%" stopColor="var(--destructive)" stopOpacity="0.4" />
					<stop offset="100%" stopColor="var(--destructive)" stopOpacity="0" />
				</linearGradient>
			</defs>
			<path d={`${path} L 100 100 L 0 100 Z`} fill="url(#errSparkFill)" />
			<path
				d={path}
				fill="none"
				stroke="var(--destructive)"
				strokeWidth="1.5"
				vectorEffect="non-scaling-stroke"
				strokeLinecap="round"
				strokeLinejoin="round"
			/>
		</svg>
	);
}

export function DashboardErrorWidget({ profile }: DashboardErrorWidgetProps) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const stats = useQuery<IErrorStatsResponse>({
		queryKey: ["admin", "logs", "stats", "dashboard"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<IErrorStatsResponse>(
				profile,
				"admin/logs/stats?hours=24&top=5",
			);
		},
		enabled: !!profile,
		refetchInterval: 60_000,
	});

	const series = useQuery<IErrorTimeseriesResponse>({
		queryKey: ["admin", "logs", "timeseries", "dashboard"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<IErrorTimeseriesResponse>(
				profile,
				"admin/logs/timeseries?hours=24&bucket=hour",
			);
		},
		enabled: !!profile,
		refetchInterval: 60_000,
	});

	const change = stats.data?.change_percent ?? null;
	const changeTone =
		change == null
			? "muted"
			: change > 5
				? "destructive"
				: change < -5
					? "good"
					: "muted";
	const ChangeIcon =
		change == null
			? ArrowRight
			: change > 0
				? ArrowUpRight
				: change < 0
					? ArrowDownRight
					: ArrowRight;

	return (
		<Card className="overflow-hidden border-destructive/20">
			<CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0 pb-3">
				<div className="space-y-1">
					<CardTitle className="flex items-center gap-2 text-base">
						<Bug className="h-4 w-4 text-destructive" />
						{t("recentErrors", "Recent Errors")}
						<Badge variant="outline" className="text-[10px]">
							24h
						</Badge>
					</CardTitle>
					<CardDescription>
						{t(
							"liveSignalOfApiFailuresAcrossThePlatform",
							"Live signal of API failures across the platform",
						)}
					</CardDescription>
				</div>
				<Button asChild size="sm" variant="outline">
					<Link href="/admin/logs">
						{t("openControlTower", "Open Control Tower")}
						<ExternalLink className="ml-1 h-3 w-3" />
					</Link>
				</Button>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="grid gap-2 sm:grid-cols-4">
					<StatTile
						label="Total"
						value={
							stats.isLoading
								? "…"
								: (stats.data?.total_errors ?? 0).toLocaleString()
						}
						icon={<Skull className="h-4 w-4" />}
						tone={stats.data?.total_errors ? "destructive" : "muted"}
					/>
					<StatTile
						label={t("server5xx", "Server (5xx)")}
						value={
							stats.isLoading
								? "…"
								: (stats.data?.server_errors ?? 0).toLocaleString()
						}
						icon={<ServerCrash className="h-4 w-4" />}
						tone={stats.data?.server_errors ? "destructive" : "muted"}
					/>
					<StatTile
						label={t("client4xx", "Client (4xx)")}
						value={
							stats.isLoading
								? "…"
								: (stats.data?.client_errors ?? 0).toLocaleString()
						}
						icon={<Zap className="h-4 w-4" />}
						tone={stats.data?.client_errors ? "warn" : "muted"}
					/>
					<StatTile
						label={t("usersHit", "Users hit")}
						value={
							stats.isLoading
								? "…"
								: (stats.data?.unique_users_affected ?? 0).toLocaleString()
						}
						icon={<UsersRound className="h-4 w-4" />}
					/>
				</div>

				<div className="rounded-lg border bg-card/50 p-3">
					<div className="mb-2 flex items-center justify-between text-xs text-muted-foreground">
						<span className="inline-flex items-center gap-1.5">
							<TrendingUp className="h-3 w-3" />{" "}
							{t("trendLast24hHourly", "Trend (last 24h, hourly)")}
						</span>
						{change != null && (
							<span
								className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] tabular-nums ${
									changeTone === "destructive"
										? "border-destructive/40 text-destructive"
										: changeTone === "good"
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

				<div>
					<div className="mb-2 flex items-center justify-between">
						<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
							{t("latest", "Latest")}
						</div>
						<Link
							href="/admin/logs"
							className="text-xs text-muted-foreground hover:text-foreground"
						>
							{t("viewAll", "View all →")}
						</Link>
					</div>
					{stats.isLoading ? (
						<div className="space-y-2">
							<Skeleton className="h-8 w-full" />
							<Skeleton className="h-8 w-full" />
							<Skeleton className="h-8 w-full" />
						</div>
					) : (stats.data?.recent.length ?? 0) === 0 ? (
						<div className="flex items-center justify-center rounded-lg border border-dashed py-6 text-sm text-muted-foreground">
							{t("allClearInTheLast24Hours", "All clear in the last 24 hours.")}
						</div>
					) : (
						<div className="space-y-1">
							{stats.data?.recent.slice(0, 5).map((err) => {
								const tone = statusCodeTone(err.status_code);
								return (
									<Link
										key={err.id}
										href={`/admin/logs?error_id=${encodeURIComponent(err.id)}`}
										className="flex items-center gap-2 rounded-md border border-transparent px-2 py-1.5 transition-colors hover:border-border hover:bg-muted/50"
									>
										<Badge
											variant={tone.variant}
											className="font-mono text-[10px]"
										>
											{err.status_code}
										</Badge>
										<TooltipProvider>
											<Tooltip>
												<TooltipTrigger asChild>
													<span className="truncate font-mono text-[11px] text-muted-foreground">
														{err.method} {err.path}
													</span>
												</TooltipTrigger>
												<TooltipContent>
													{err.method} {err.path}
												</TooltipContent>
											</Tooltip>
										</TooltipProvider>
										<span className="ml-auto inline-flex items-center gap-2">
											{err.user_id ? (
												<UserPill userId={err.user_id} compact muted />
											) : null}
											<RelativeTime
												value={err.created_at}
												className="text-[11px] text-muted-foreground"
											/>
										</span>
									</Link>
								);
							})}
						</div>
					)}
				</div>

				<div className="grid gap-3 sm:grid-cols-2">
					<TopList
						title={t("topErrorCodes", "Top error codes")}
						buckets={stats.data?.top_codes ?? []}
						loading={stats.isLoading}
						hrefBuilder={(b) =>
							`/admin/logs?public_code=${encodeURIComponent(b.key)}`
						}
					/>
					<TopList
						title={t("topPaths", "Top paths")}
						buckets={stats.data?.top_paths ?? []}
						loading={stats.isLoading}
						hrefBuilder={(b) => `/admin/logs?path=${encodeURIComponent(b.key)}`}
					/>
				</div>
			</CardContent>
		</Card>
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
								<span className="w-24 truncate text-xs font-medium group-hover:text-primary">
									{b.label}
								</span>
								<div className="relative flex-1 overflow-hidden rounded-full bg-muted h-1.5">
									<div
										className="h-full rounded-full bg-destructive/60"
										style={{ width: `${(b.count / max) * 100}%` }}
									/>
								</div>
								<span className="w-8 text-right text-[11px] tabular-nums text-muted-foreground">
									{b.count}
								</span>
							</Link>
						</li>
					))}
				</ul>
			)}
		</div>
	);
}

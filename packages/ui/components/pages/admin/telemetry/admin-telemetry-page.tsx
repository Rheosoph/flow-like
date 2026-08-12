"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
	Activity,
	ArrowDownRight,
	ArrowRight,
	ArrowUpRight,
	Bug,
	Copy,
	GitBranch,
	Layers,
	Lock,
	MonitorSmartphone,
	RefreshCw,
	Zap,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { Area, AreaChart, CartesianGrid, XAxis, YAxis } from "recharts";
import { toast } from "sonner";
import { useInvoke } from "../../../../hooks/use-invoke";
import { GlobalPermission } from "../../../../lib/permission/global-permission";
import { useBackend } from "../../../../state/backend-state";
import {
	Badge,
	Button,
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
	RelativeTime,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	Skeleton,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../../../ui";
import { EngagementSection } from "./engagement-section";
import { FlowpilotSection } from "./flowpilot-section";
import { TelemetryGranularityNotice } from "./granularity-notice";
import { LlmSection } from "./llm-section";
import { PerformanceSection } from "./performance-section";
import { ReleaseHealthSection } from "./release-health-section";
import type {
	ITelemetryEventRow,
	ITelemetryEventsResponse,
	ITelemetryOverviewResponse,
	ITelemetryTimeseriesResponse,
} from "./types";

const HOUR_OPTIONS: { value: number; label: string }[] = [
	{ value: 6, label: "Last 6 hours" },
	{ value: 24, label: "Last 24 hours" },
	{ value: 168, label: "Last 7 days" },
	{ value: 720, label: "Last 30 days" },
	{ value: 2160, label: "Last 90 days" },
];

const SOURCE_OPTIONS = ["desktop", "web", "desktop_core", "backend"] as const;

const DAY_VALUES = [30, 60, 90] as const;

const ALL_EVENTS = "ALL";

interface FilterState {
	hours: number;
	name: string;
	source: string;
	days: number;
}

function decodeFiltersFromSearch(params: {
	get(name: string): string | null;
}): FilterState {
	const source = params.get("source") ?? "";
	const days = Number.parseInt(params.get("days") ?? "30", 10);
	return {
		hours: Number.parseInt(params.get("hours") ?? "24", 10) || 24,
		name: params.get("name") ?? "",
		source: (SOURCE_OPTIONS as readonly string[]).includes(source)
			? source
			: "all",
		days: (DAY_VALUES as readonly number[]).includes(days) ? days : 30,
	};
}

function encodeFiltersToSearch(filters: FilterState): string {
	const p = new URLSearchParams();
	if (filters.hours !== 24) p.set("hours", String(filters.hours));
	if (filters.name) p.set("name", filters.name);
	if (filters.source !== "all") p.set("source", filters.source);
	if (filters.days !== 30) p.set("days", String(filters.days));
	return p.toString();
}

function countryLabel(code: string) {
	if (!/^[A-Z]{2}$/.test(code)) return code;
	const flag = String.fromCodePoint(
		...[...code].map((c) => 0x1f1a5 + c.charCodeAt(0)),
	);
	return `${flag} ${code}`;
}

const timeseriesChartConfig = {
	events: {
		label: "Events",
		color: "var(--chart-1)",
	},
	installs: {
		label: "Active installs",
		color: "var(--chart-2)",
	},
} satisfies ChartConfig;

function formatTick(value: string, bucket: string) {
	const d = new Date(value);
	if (Number.isNaN(d.getTime())) return value;
	if (bucket === "minute") {
		return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
	}
	if (bucket === "hour") {
		return d.toLocaleTimeString([], { hour: "2-digit" });
	}
	return d.toLocaleDateString([], { month: "short", day: "numeric" });
}

function StatHero({
	label,
	value,
	icon,
	tone = "muted",
	hint,
}: {
	label: string;
	value: string;
	icon: React.ReactNode;
	tone?: "muted" | "good";
	hint?: React.ReactNode;
}) {
	const ring =
		tone === "good"
			? "border-emerald-500/30 bg-emerald-500/5"
			: "border-border bg-muted/40";
	return (
		<div className={`rounded-xl border ${ring} p-4`}>
			<div className="flex items-center justify-between text-muted-foreground">
				<span className="text-xs uppercase tracking-wide">{label}</span>
				{icon}
			</div>
			<div className="mt-1 truncate text-2xl font-bold tabular-nums">
				{value}
			</div>
			{hint ? (
				<div className="mt-1 text-xs text-muted-foreground">{hint}</div>
			) : null}
		</div>
	);
}

function TelemetryChart({
	points,
	bucket,
}: {
	points: { ts: string; count: number; installs: number }[];
	bucket: string;
}) {
	const data = useMemo(
		() =>
			points.map((p) => ({
				ts: p.ts,
				events: p.count,
				installs: p.installs,
			})),
		[points],
	);

	if (data.length === 0) {
		return (
			<div className="flex h-64 items-center justify-center rounded-lg border border-dashed text-sm text-muted-foreground">
				No telemetry in the selected window.
			</div>
		);
	}

	return (
		<ChartContainer config={timeseriesChartConfig} className="h-64 w-full">
			<AreaChart data={data} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
				<defs>
					<linearGradient id="telemetryChartEvents" x1="0" y1="0" x2="0" y2="1">
						<stop
							offset="0%"
							stopColor="var(--color-events)"
							stopOpacity={0.3}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-events)"
							stopOpacity={0.03}
						/>
					</linearGradient>
					<linearGradient
						id="telemetryChartInstalls"
						x1="0"
						y1="0"
						x2="0"
						y2="1"
					>
						<stop
							offset="0%"
							stopColor="var(--color-installs)"
							stopOpacity={0.2}
						/>
						<stop
							offset="100%"
							stopColor="var(--color-installs)"
							stopOpacity={0.02}
						/>
					</linearGradient>
				</defs>
				<CartesianGrid strokeDasharray="3 3" vertical={false} opacity={0.4} />
				<XAxis
					dataKey="ts"
					tickLine={false}
					axisLine={false}
					tick={{ fontSize: 11 }}
					tickFormatter={(v) => formatTick(v, bucket)}
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
							labelFormatter={(value) => formatTick(value as string, bucket)}
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
					dataKey="events"
					stroke="var(--color-events)"
					fill="url(#telemetryChartEvents)"
					strokeWidth={2}
				/>
				<Area
					type="monotone"
					dataKey="installs"
					stroke="var(--color-installs)"
					fill="url(#telemetryChartInstalls)"
					strokeWidth={2}
				/>
			</AreaChart>
		</ChartContainer>
	);
}

function BreakdownCard({
	title,
	buckets,
	loading,
	onPick,
}: {
	title: string;
	buckets: { key: string; label: string; count: number }[];
	loading: boolean;
	onPick?: (b: { key: string; label: string; count: number }) => void;
}) {
	const max = Math.max(1, ...buckets.map((b) => b.count));
	return (
		<Card>
			<CardHeader className="pb-2">
				<CardTitle className="text-sm">{title}</CardTitle>
			</CardHeader>
			<CardContent>
				{loading ? (
					<div className="space-y-1.5">
						<Skeleton className="h-4 w-full" />
						<Skeleton className="h-4 w-full" />
						<Skeleton className="h-4 w-full" />
					</div>
				) : buckets.length === 0 ? (
					<div className="flex items-center justify-center rounded-lg border border-dashed py-6 text-xs text-muted-foreground">
						No data in the selected window.
					</div>
				) : (
					<ul className="space-y-1.5">
						{buckets.map((b) => {
							const row = (
								<>
									<span className="w-32 truncate text-left font-mono text-xs font-medium group-hover:text-primary">
										{b.label}
									</span>
									<div className="relative flex-1 overflow-hidden rounded-full bg-muted h-2">
										<div
											className="h-full rounded-full bg-primary/60"
											style={{ width: `${(b.count / max) * 100}%` }}
										/>
									</div>
									<span className="w-14 text-right text-[11px] tabular-nums text-muted-foreground">
										{b.count.toLocaleString()}
									</span>
								</>
							);
							return (
								<li key={b.key}>
									{onPick ? (
										<button
											type="button"
											onClick={() => onPick(b)}
											className="group flex w-full items-center gap-2 rounded px-1 py-0.5 hover:bg-muted/60"
										>
											{row}
										</button>
									) : (
										<div className="group flex items-center gap-2 rounded px-1 py-0.5">
											{row}
										</div>
									)}
								</li>
							);
						})}
					</ul>
				)}
			</CardContent>
		</Card>
	);
}

function safeStringify(v: unknown) {
	try {
		return JSON.stringify(v, null, 2);
	} catch {
		return String(v);
	}
}

function TelemetryEventSheet({
	event,
	open,
	onOpenChange,
}: {
	event: ITelemetryEventRow | null;
	open: boolean;
	onOpenChange: (open: boolean) => void;
}) {
	const propsString = useMemo(() => {
		if (!event?.props) return null;
		return safeStringify(event.props);
	}, [event?.props]);

	return (
		<Sheet open={open} onOpenChange={onOpenChange}>
			<SheetContent className="w-full overflow-y-auto sm:max-w-xl">
				<SheetHeader>
					<SheetTitle className="flex items-center gap-2">
						<Badge variant="outline" className="font-mono text-[11px]">
							{event?.source ?? ""}
						</Badge>
						<span className="font-mono text-sm">{event?.name}</span>
					</SheetTitle>
					<SheetDescription className="font-mono text-xs">
						{event?.id}
					</SheetDescription>
				</SheetHeader>

				{event ? (
					<div className="space-y-4 px-4 pb-6">
						<div className="space-y-2 rounded-lg border bg-card/40 p-3 text-sm">
							<div className="flex flex-wrap items-center gap-2">
								<RelativeTime
									value={event.createdAt}
									className="text-xs text-muted-foreground"
								/>
								<Badge variant="secondary" className="text-[10px]">
									{event.platform ?? "unknown"}
								</Badge>
								{event.appVersion ? (
									<Badge variant="outline" className="font-mono text-[10px]">
										{event.appVersion}
									</Badge>
								) : null}
							</div>
							<div className="flex items-center gap-2">
								<span className="text-xs text-muted-foreground">Install</span>
								<code className="flex-1 truncate rounded bg-muted px-1.5 py-0.5 font-mono text-[11px]">
									{event.anonId}
								</code>
							</div>
							{event.clientTs ? (
								<div className="flex items-center gap-2">
									<span className="text-xs text-muted-foreground">
										Client time
									</span>
									<code className="rounded bg-muted px-1.5 py-0.5 font-mono text-[11px]">
										{event.clientTs}
									</code>
								</div>
							) : null}
						</div>

						{propsString ? (
							<div className="space-y-1">
								<div className="flex items-center justify-between">
									<div className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
										Properties
									</div>
									<Button
										variant="ghost"
										size="sm"
										onClick={() => {
											navigator.clipboard
												.writeText(propsString)
												.catch(() => null);
											toast.success("Properties copied");
										}}
									>
										<Copy className="mr-1 h-3 w-3" />
										Copy
									</Button>
								</div>
								<pre className="max-h-96 overflow-auto rounded-lg border bg-muted/40 p-3 font-mono text-[11px] leading-relaxed">
									{propsString}
								</pre>
							</div>
						) : (
							<div className="flex items-center justify-center rounded-lg border border-dashed py-6 text-xs text-muted-foreground">
								No properties attached to this event.
							</div>
						)}

						<Separator />

						<p className="text-xs text-muted-foreground">
							Telemetry is anonymous: the install id is a random identifier and
							no user identity or IP address is ever stored.
						</p>
					</div>
				) : null}
			</SheetContent>
		</Sheet>
	);
}

interface AdminTelemetryPageProps {
	basePath?: string;
}

export function AdminTelemetryPage({
	basePath = "/admin/telemetry",
}: Readonly<AdminTelemetryPageProps>) {
	const backend = useBackend();
	const auth = useAuth();
	const router = useRouter();
	const queryClient = useQueryClient();
	const searchParams = useSearchParams();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		[auth?.user?.profile?.sub, auth?.isAuthenticated],
	);

	const initialFilters = useMemo(
		() => decodeFiltersFromSearch(searchParams ?? new URLSearchParams()),
		[searchParams],
	);

	const [filters, setFilters] = useState<FilterState>(initialFilters);
	const [page, setPage] = useState(0);
	const [selectedEvent, setSelectedEvent] = useState<ITelemetryEventRow | null>(
		null,
	);
	const [showDetail, setShowDetail] = useState(false);

	const pageSize = 50;

	const overview = useQuery<ITelemetryOverviewResponse>({
		queryKey: ["admin", "telemetry", "overview", filters.hours],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryOverviewResponse>(
				profile.data,
				`admin/telemetry/overview?hours=${filters.hours}`,
			);
		},
		enabled: !!profile.data,
	});

	const series = useQuery<ITelemetryTimeseriesResponse>({
		queryKey: [
			"admin",
			"telemetry",
			"timeseries",
			filters.hours,
			filters.name,
			filters.source,
		],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const p = new URLSearchParams({ hours: String(filters.hours) });
			if (filters.name) p.set("name", filters.name);
			if (filters.source !== "all") p.set("source", filters.source);
			return backend.apiState.get<ITelemetryTimeseriesResponse>(
				profile.data,
				`admin/telemetry/timeseries?${p.toString()}`,
			);
		},
		enabled: !!profile.data,
	});

	const events = useQuery<ITelemetryEventsResponse>({
		queryKey: [
			"admin",
			"telemetry",
			"events",
			page,
			pageSize,
			filters.name,
			filters.source,
		],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const p = new URLSearchParams({
				page: String(page),
				page_size: String(pageSize),
			});
			if (filters.name) p.set("name", filters.name);
			if (filters.source !== "all") p.set("source", filters.source);
			return backend.apiState.get<ITelemetryEventsResponse>(
				profile.data,
				`admin/telemetry/events?${p.toString()}`,
			);
		},
		enabled: !!profile.data,
	});

	useEffect(() => {
		const qs = encodeFiltersToSearch(filters);
		router.replace(qs ? `${basePath}?${qs}` : basePath);
	}, [filters, router, basePath]);

	const setFilterValue = useCallback(
		<K extends keyof FilterState>(key: K, value: FilterState[K]) => {
			setFilters((prev) => ({ ...prev, [key]: value }));
			setPage(0);
		},
		[],
	);

	const refresh = useCallback(() => {
		queryClient.invalidateQueries({ queryKey: ["admin", "telemetry"] });
	}, [queryClient]);

	const onDaysChange = useCallback(
		(days: number) => setFilterValue("days", days),
		[setFilterValue],
	);

	const openEventDetail = useCallback((event: ITelemetryEventRow) => {
		setSelectedEvent(event);
		setShowDetail(true);
	}, []);

	const change = useMemo(() => {
		if (!overview.data) return null;
		const { totalEvents, previousTotalEvents } = overview.data;
		if (previousTotalEvents <= 0) return null;
		return ((totalEvents - previousTotalEvents) / previousTotalEvents) * 100;
	}, [overview.data]);

	const eventNameOptions = useMemo(() => {
		const names = (overview.data?.topEvents ?? []).map((e) => e.name);
		if (filters.name && !names.includes(filters.name)) {
			names.push(filters.name);
		}
		return names;
	}, [overview.data?.topEvents, filters.name]);

	const perms = useMemo(
		() => new GlobalPermission(info.data?.permission ?? 0),
		[info.data?.permission],
	);
	const hasAccess = perms.hasPermission(GlobalPermission.Admin);

	if (info.isLoading) {
		return (
			<main className="flex h-full min-h-0 w-full grow flex-col bg-background p-6">
				<Skeleton className="h-12 w-72" />
				<div className="mt-4 grid grid-cols-4 gap-3">
					<Skeleton className="h-24" />
					<Skeleton className="h-24" />
					<Skeleton className="h-24" />
					<Skeleton className="h-24" />
				</div>
			</main>
		);
	}

	if (!hasAccess) {
		return (
			<main className="flex h-full w-full items-center justify-center bg-background p-6">
				<Card className="max-w-md text-center">
					<CardHeader>
						<CardTitle className="flex items-center justify-center gap-2 text-base">
							<Lock className="h-4 w-4" />
							Insufficient permissions
						</CardTitle>
						<CardDescription>
							You need the <b>Admin</b> permission to view telemetry.
						</CardDescription>
					</CardHeader>
				</Card>
			</main>
		);
	}

	const totalPages = Math.max(
		1,
		Math.ceil((events.data?.total ?? 0) / pageSize),
	);
	const topSource = overview.data?.sources[0];
	const ChangeIcon =
		change == null
			? ArrowRight
			: change > 0
				? ArrowUpRight
				: change < 0
					? ArrowDownRight
					: ArrowRight;
	const hourLabel =
		HOUR_OPTIONS.find((o) => o.value === filters.hours)?.label ??
		`Last ${filters.hours} hours`;

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-7xl space-y-6">
					<div className="flex flex-wrap items-start justify-between gap-3">
						<div>
							<h1 className="flex items-center gap-2 text-3xl font-bold">
								<Activity className="h-7 w-7 text-primary" />
								Telemetry
								<TelemetryGranularityNotice response={overview.data} />
							</h1>
							<p className="text-muted-foreground">
								Anonymous opt-in product metrics — no user identity, no IP
								addresses.
							</p>
						</div>
						<div className="flex flex-wrap items-center gap-2">
							<Select
								value={String(filters.hours)}
								onValueChange={(v) =>
									setFilterValue("hours", Number.parseInt(v, 10))
								}
							>
								<SelectTrigger className="w-40">
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{HOUR_OPTIONS.map((o) => (
										<SelectItem key={o.value} value={String(o.value)}>
											{o.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<Select
								value={filters.name || ALL_EVENTS}
								onValueChange={(v) =>
									setFilterValue("name", v === ALL_EVENTS ? "" : v)
								}
							>
								<SelectTrigger className="w-52">
									<SelectValue placeholder="Event" />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value={ALL_EVENTS}>All events</SelectItem>
									{eventNameOptions.map((name) => (
										<SelectItem key={name} value={name}>
											{name}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<Select
								value={filters.source}
								onValueChange={(v) => setFilterValue("source", v)}
							>
								<SelectTrigger className="w-40">
									<SelectValue placeholder="Source" />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="all">All sources</SelectItem>
									{SOURCE_OPTIONS.map((source) => (
										<SelectItem key={source} value={source}>
											{source}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<Button asChild variant="outline" size="sm">
								<Link href={`${basePath}/issues`}>
									<Bug className="mr-1 h-3.5 w-3.5" />
									Issues
								</Link>
							</Button>
							<Button asChild variant="outline" size="sm">
								<Link href={`${basePath}/traces`}>
									<GitBranch className="mr-1 h-3.5 w-3.5" />
									Traces
								</Link>
							</Button>
							<Button variant="outline" size="sm" onClick={refresh}>
								<RefreshCw className="mr-1 h-3.5 w-3.5" />
								Refresh
							</Button>
						</div>
					</div>

					<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
						<StatHero
							label="Total events"
							value={
								overview.isLoading
									? "…"
									: (overview.data?.totalEvents ?? 0).toLocaleString()
							}
							icon={<Zap className="h-4 w-4" />}
							hint={hourLabel}
						/>
						<StatHero
							label="Active installs"
							value={
								overview.isLoading
									? "…"
									: (overview.data?.activeInstalls ?? 0).toLocaleString()
							}
							icon={<MonitorSmartphone className="h-4 w-4" />}
							hint="Distinct anonymous ids"
						/>
						<StatHero
							label="Change"
							value={
								overview.isLoading
									? "…"
									: change == null
										? "—"
										: `${change >= 0 ? "+" : ""}${change.toFixed(1)}%`
							}
							icon={<ChangeIcon className="h-4 w-4" />}
							tone={change != null && change > 5 ? "good" : "muted"}
							hint="vs previous period"
						/>
						<StatHero
							label="Top source"
							value={overview.isLoading ? "…" : (topSource?.source ?? "—")}
							icon={<Layers className="h-4 w-4" />}
							hint={
								topSource
									? `${topSource.count.toLocaleString()} events`
									: "No events yet"
							}
						/>
					</div>

					<Card>
						<CardHeader className="pb-3">
							<CardTitle className="flex items-center gap-2 text-base">
								<Activity className="h-4 w-4" />
								Events over time
								<TelemetryGranularityNotice response={series.data} />
							</CardTitle>
							<CardDescription>
								Events and active installs bucketed by{" "}
								<span className="font-mono">
									{series.data?.bucket ?? "auto"}
								</span>{" "}
								over the selected window.
							</CardDescription>
						</CardHeader>
						<CardContent>
							{series.isLoading ? (
								<Skeleton className="h-64 w-full" />
							) : (
								<TelemetryChart
									points={series.data?.points ?? []}
									bucket={series.data?.bucket ?? "hour"}
								/>
							)}
						</CardContent>
					</Card>

					<div className="grid gap-4 lg:grid-cols-2">
						<BreakdownCard
							title="Top events"
							buckets={
								overview.data?.topEvents.map((e) => ({
									key: e.name,
									label: e.name,
									count: e.count,
								})) ?? []
							}
							loading={overview.isLoading}
							onPick={(b) => setFilterValue("name", b.key)}
						/>
						<BreakdownCard
							title="Sources"
							buckets={
								overview.data?.sources.map((s) => ({
									key: s.source,
									label: s.source,
									count: s.count,
								})) ?? []
							}
							loading={overview.isLoading}
							onPick={(b) => {
								if ((SOURCE_OPTIONS as readonly string[]).includes(b.key)) {
									setFilterValue("source", b.key);
								}
							}}
						/>
						<BreakdownCard
							title="Platforms"
							buckets={
								overview.data?.platforms.map((p) => ({
									key: p.platform,
									label: p.platform,
									count: p.count,
								})) ?? []
							}
							loading={overview.isLoading}
						/>
						<BreakdownCard
							title="Versions"
							buckets={
								overview.data?.versions.map((v) => ({
									key: v.appVersion,
									label: v.appVersion,
									count: v.count,
								})) ?? []
							}
							loading={overview.isLoading}
						/>
						<BreakdownCard
							title="Countries"
							buckets={(overview.data?.countries ?? []).map((c) => ({
								key: c.country,
								label: countryLabel(c.country),
								count: c.count,
							}))}
							loading={overview.isLoading}
						/>
					</div>

					<EngagementSection
						profile={profile.data}
						days={filters.days}
						onDaysChange={onDaysChange}
					/>

					<FlowpilotSection profile={profile.data} hours={filters.hours} />

					<ReleaseHealthSection
						profile={profile.data}
						hours={filters.hours}
						source={filters.source}
					/>

					<PerformanceSection
						profile={profile.data}
						hours={filters.hours}
						source={filters.source}
					/>

					<LlmSection
						profile={profile.data}
						hours={filters.hours}
						source={filters.source}
					/>

					<Card>
						<CardHeader className="pb-3">
							<div className="flex flex-wrap items-center justify-between gap-2">
								<CardTitle className="text-base">Recent events</CardTitle>
								<CardDescription>
									{(events.data?.total ?? 0).toLocaleString()} matching events
								</CardDescription>
							</div>
						</CardHeader>
						<CardContent className="p-0">
							{events.isLoading ? (
								<div className="space-y-1.5 p-4">
									<Skeleton className="h-10 w-full" />
									<Skeleton className="h-10 w-full" />
									<Skeleton className="h-10 w-full" />
								</div>
							) : (events.data?.events.length ?? 0) === 0 ? (
								<div className="flex h-40 items-center justify-center rounded-lg border border-dashed m-4 text-sm text-muted-foreground">
									No events match the current filters.
								</div>
							) : (
								<Table>
									<TableHeader>
										<TableRow>
											<TableHead>Time</TableHead>
											<TableHead>Name</TableHead>
											<TableHead>Source</TableHead>
											<TableHead>Install</TableHead>
											<TableHead>Platform</TableHead>
											<TableHead>Version</TableHead>
										</TableRow>
									</TableHeader>
									<TableBody>
										{events.data?.events.map((event) => (
											<TableRow
												key={event.id}
												className="cursor-pointer"
												onClick={() => openEventDetail(event)}
											>
												<TableCell>
													<RelativeTime
														value={event.createdAt}
														className="text-xs text-muted-foreground"
													/>
												</TableCell>
												<TableCell className="font-mono text-xs">
													{event.name}
												</TableCell>
												<TableCell>
													<Badge
														variant="outline"
														className="font-mono text-[10px]"
													>
														{event.source}
													</Badge>
												</TableCell>
												<TableCell>
													<span
														className="block max-w-[8rem] truncate font-mono text-[11px] text-muted-foreground"
														title={event.anonId}
													>
														{event.anonId}
													</span>
												</TableCell>
												<TableCell className="text-xs text-muted-foreground">
													{event.platform ?? "unknown"}
												</TableCell>
												<TableCell className="font-mono text-xs text-muted-foreground">
													{event.appVersion ?? "—"}
												</TableCell>
											</TableRow>
										))}
									</TableBody>
								</Table>
							)}
						</CardContent>
					</Card>

					{totalPages > 1 && (
						<div className="flex items-center justify-between">
							<div className="text-sm text-muted-foreground">
								Page {page + 1} of {totalPages}
							</div>
							<div className="flex gap-2">
								<Button
									variant="outline"
									size="sm"
									onClick={() => setPage((p) => Math.max(0, p - 1))}
									disabled={page === 0}
								>
									Previous
								</Button>
								<Button
									variant="outline"
									size="sm"
									onClick={() =>
										setPage((p) => Math.min(totalPages - 1, p + 1))
									}
									disabled={page >= totalPages - 1}
								>
									Next
								</Button>
							</div>
						</div>
					)}
				</div>
			</div>

			<TelemetryEventSheet
				event={selectedEvent}
				open={showDetail}
				onOpenChange={setShowDetail}
			/>
		</main>
	);
}

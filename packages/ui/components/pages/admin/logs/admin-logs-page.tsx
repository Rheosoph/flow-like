"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useDebounce } from "@uidotdev/usehooks";
import {
	Activity,
	AlertTriangle,
	ArrowDownRight,
	ArrowRight,
	ArrowUpRight,
	Bug,
	Copy,
	Filter,
	Lock,
	RefreshCw,
	Search,
	ServerCrash,
	UsersRound,
	X,
	Zap,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
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
	Input,
	RelativeTime,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "../../../ui";
import { AuditChainExplorer } from "./audit-chain-explorer";
import { ErrorChart } from "./error-chart";
import { ErrorDetailSheet } from "./error-detail-sheet";
import type {
	IErrorStatsResponse,
	IErrorTimeseriesResponse,
	IListErrorsResponse,
} from "./types";
import { statusCodeTone } from "./types";
import { UserPill } from "./user-pill";

type Severity = "all" | "client" | "server";

const HOUR_OPTIONS: { value: number; label: string }[] = [
	{ value: 1, label: "Last hour" },
	{ value: 6, label: "Last 6 hours" },
	{ value: 24, label: "Last 24 hours" },
	{ value: 72, label: "Last 3 days" },
	{ value: 168, label: "Last 7 days" },
	{ value: 720, label: "Last 30 days" },
];

interface FilterState {
	query: string;
	error_id: string;
	method: string;
	path: string;
	public_code: string;
	user_id: string;
	severity: Severity;
	hours: number;
}

const EMPTY_FILTERS: FilterState = {
	query: "",
	error_id: "",
	method: "",
	path: "",
	public_code: "",
	user_id: "",
	severity: "all",
	hours: 24,
};

function decodeFiltersFromSearch(
	params: URLSearchParams | ReadonlyURLSearchParams,
): FilterState {
	return {
		query: params.get("q") ?? "",
		error_id: params.get("error_id") ?? "",
		method: params.get("method") ?? "",
		path: params.get("path") ?? "",
		public_code: params.get("public_code") ?? "",
		user_id: params.get("user_id") ?? "",
		severity: ((): Severity => {
			const v = params.get("severity");
			return v === "client" || v === "server" ? v : "all";
		})(),
		hours: Number.parseInt(params.get("hours") ?? "24", 10) || 24,
	};
}

interface ReadonlyURLSearchParams {
	get(name: string): string | null;
}

function encodeFiltersToSearch(filters: FilterState, tab: string): string {
	const p = new URLSearchParams();
	if (filters.query) p.set("q", filters.query);
	if (filters.error_id) p.set("error_id", filters.error_id);
	if (filters.method) p.set("method", filters.method);
	if (filters.path) p.set("path", filters.path);
	if (filters.public_code) p.set("public_code", filters.public_code);
	if (filters.user_id) p.set("user_id", filters.user_id);
	if (filters.severity !== "all") p.set("severity", filters.severity);
	if (filters.hours !== 24) p.set("hours", String(filters.hours));
	if (tab !== "errors") p.set("tab", tab);
	return p.toString();
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
	tone?: "muted" | "destructive" | "warn" | "good";
	hint?: React.ReactNode;
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
		<div className={`rounded-xl border ${ring} p-4`}>
			<div className="flex items-center justify-between text-muted-foreground">
				<span className="text-xs uppercase tracking-wide">{label}</span>
				{icon}
			</div>
			<div className="mt-1 text-2xl font-bold tabular-nums">{value}</div>
			{hint ? (
				<div className="mt-1 text-xs text-muted-foreground">{hint}</div>
			) : null}
		</div>
	);
}

function ChangePill({ change }: { change: number | null | undefined }) {
	const { t } = useTranslation("admin");
	if (change == null) {
		return (
			<span className="inline-flex items-center gap-1 text-muted-foreground">
				<ArrowRight className="h-3 w-3" />
				{t('noPriorData', 'No prior data')}
			</span>
		);
	}
	const Icon =
		change > 0 ? ArrowUpRight : change < 0 ? ArrowDownRight : ArrowRight;
	const tone =
		change > 5
			? "text-destructive"
			: change < -5
				? "text-emerald-600 dark:text-emerald-400"
				: "text-muted-foreground";
	return (
		<span className={`inline-flex items-center gap-1 ${tone}`}>
			<Icon className="h-3 w-3" />
			{change >= 0 ? "+" : ""}
			{change.toFixed(1)}{t('vsPriorWindow', '% vs prior window')}
		</span>
	);
}

function ActiveFilterChip({
	label,
	value,
	onClear,
}: {
	label: string;
	value: string;
	onClear: () => void;
}) {
	const { t } = useTranslation("admin");
	return (
		<Badge
			variant="secondary"
			className="cursor-pointer gap-1 pr-1 hover:bg-secondary/80"
			onClick={onClear}
		>
			<span className="text-[11px] text-muted-foreground">{`${label}:`}</span>
			<span className="max-w-[140px] truncate font-mono">{value}</span>
			<X className="h-3 w-3" />
		</Badge>
	);
}

interface AdminLogsPageProps {
	basePath?: string;
}

export function AdminLogsPage({
	basePath = "/admin/logs",
}: Readonly<AdminLogsPageProps>) {
	const { t } = useTranslation("admin");
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
	const initialTab = searchParams?.get("tab") ?? "errors";

	const [filters, setFilters] = useState<FilterState>(initialFilters);
	const [tab, setTab] = useState(initialTab);
	const [page, setPage] = useState(1);
	const [selectedError, setSelectedError] = useState<string | null>(
		initialFilters.error_id || null,
	);
	const [showDetail, setShowDetail] = useState<boolean>(
		Boolean(initialFilters.error_id),
	);

	const debouncedQuery = useDebounce(filters.query, 300);
	const limit = 25;

	const queryParams = useMemo(() => {
		const params: Record<string, string | number> = {
			offset: (page - 1) * limit,
			limit,
			hours: filters.hours,
		};
		if (debouncedQuery) params.query = debouncedQuery;
		if (filters.error_id) params.error_id = filters.error_id;
		if (filters.method) params.method = filters.method;
		if (filters.path) params.path = filters.path;
		if (filters.public_code) params.public_code = filters.public_code;
		if (filters.user_id) params.user_id = filters.user_id;
		if (filters.severity !== "all") params.severity = filters.severity;
		return params;
	}, [
		debouncedQuery,
		filters.error_id,
		filters.hours,
		filters.method,
		filters.path,
		filters.public_code,
		filters.user_id,
		filters.severity,
		page,
	]);

	const errorsQuery = useQuery<IListErrorsResponse>({
		queryKey: ["admin", "logs", "errors", queryParams],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			const qs = new URLSearchParams(
				Object.entries(queryParams).map(([k, v]) => [k, String(v)]),
			).toString();
			return backend.apiState.get<IListErrorsResponse>(
				profile.data,
				`admin/logs/errors?${qs}`,
			);
		},
		enabled: !!profile.data,
	});

	const stats = useQuery<IErrorStatsResponse>({
		queryKey: ["admin", "logs", "stats", filters.hours],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<IErrorStatsResponse>(
				profile.data,
				`admin/logs/stats?hours=${filters.hours}&top=10`,
			);
		},
		enabled: !!profile.data,
	});

	const series = useQuery<IErrorTimeseriesResponse>({
		queryKey: ["admin", "logs", "timeseries", filters.hours],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<IErrorTimeseriesResponse>(
				profile.data,
				`admin/logs/timeseries?hours=${filters.hours}`,
			);
		},
		enabled: !!profile.data,
	});

	useEffect(() => {
		const qs = encodeFiltersToSearch(filters, tab);
		router.replace(qs ? `${basePath}?${qs}` : basePath);
	}, [filters, tab, router, basePath]);

	const totalPages = Math.max(
		1,
		Math.ceil((errorsQuery.data?.total ?? 0) / limit),
	);

	const setFilterValue = useCallback(
		<K extends keyof FilterState>(key: K, value: FilterState[K]) => {
			setFilters((prev) => ({ ...prev, [key]: value }));
			setPage(1);
		},
		[],
	);

	const handleApplyFilter = useCallback(
		(filter: { key: string; value: string }) => {
			if (filter.key in EMPTY_FILTERS) {
				setFilterValue(filter.key as keyof FilterState, filter.value as never);
			}
			setShowDetail(false);
		},
		[setFilterValue],
	);

	const handleClearAll = useCallback(() => {
		setFilters({ ...EMPTY_FILTERS });
		setPage(1);
	}, []);

	const refresh = useCallback(() => {
		queryClient.invalidateQueries({ queryKey: ["admin", "logs"] });
	}, [queryClient]);

	const openErrorDetail = useCallback((id: string) => {
		setSelectedError(id);
		setShowDetail(true);
	}, []);

	const perms = useMemo(
		() => new GlobalPermission(info.data?.permission ?? 0),
		[info.data?.permission],
	);
	const hasAccess = perms.hasPermission(GlobalPermission.ReadLogs);

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
							{t('insufficientPermissions', 'Insufficient permissions')}
						</CardTitle>
						<CardDescription><Trans i18nKey="youNeedTheBreadlogsbPermissionToViewTheControlTower">You need the <b>ReadLogs</b> permission to view the control tower.</Trans></CardDescription>
					</CardHeader>
				</Card>
			</main>
		);
	}

	const activeFilterEntries: {
		key: keyof FilterState;
		label: string;
		value: string;
	}[] = [];
	if (filters.error_id)
		activeFilterEntries.push({
			key: "error_id",
			label: "id",
			value: filters.error_id,
		});
	if (filters.method)
		activeFilterEntries.push({
			key: "method",
			label: "method",
			value: filters.method,
		});
	if (filters.path)
		activeFilterEntries.push({
			key: "path",
			label: "path",
			value: filters.path,
		});
	if (filters.public_code)
		activeFilterEntries.push({
			key: "public_code",
			label: "code",
			value: filters.public_code,
		});
	if (filters.user_id)
		activeFilterEntries.push({
			key: "user_id",
			label: "user",
			value: filters.user_id,
		});
	if (filters.severity !== "all")
		activeFilterEntries.push({
			key: "severity",
			label: "severity",
			value: filters.severity,
		});

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-7xl space-y-6">
					<div className="flex flex-wrap items-start justify-between gap-3">
						<div>
							<h1 className="flex items-center gap-2 text-3xl font-bold">
								<Activity className="h-7 w-7 text-destructive" />
								{t('controlTower', 'Control Tower')}
							</h1>
							<p className="text-muted-foreground">
								{t('liveObservabilityForApiErrorsAndTheCryptographicAuditChain', "Live observability for API errors and the cryptographic audit chain.")}
							</p>
						</div>
						<div className="flex items-center gap-2">
							<Select
								value={String(filters.hours)}
								onValueChange={(v) =>
									setFilterValue("hours", Number.parseInt(v, 10))
								}
							>
								<SelectTrigger className="w-44">
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
							<Button variant="outline" size="sm" onClick={refresh}>
								<RefreshCw className="mr-1 h-3.5 w-3.5" />
								{t('refresh', 'Refresh')}
							</Button>
						</div>
					</div>

					<Tabs value={tab} onValueChange={setTab}>
						<TabsList>
							<TabsTrigger value="errors" className="gap-1.5">
								<Bug className="h-3.5 w-3.5" /> {t('errors', 'Errors')}
							</TabsTrigger>
							<TabsTrigger value="audit" className="gap-1.5">
								<Activity className="h-3.5 w-3.5" /> {t('auditChain', 'Audit chain')}
							</TabsTrigger>
						</TabsList>

						<TabsContent value="errors" className="space-y-6">
							<div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
								<StatHero
									label={t('totalErrors', 'Total errors')}
									value={
										stats.isLoading
											? "…"
											: (stats.data?.total_errors ?? 0).toLocaleString()
									}
									icon={<Bug className="h-4 w-4" />}
									tone={stats.data?.total_errors ? "destructive" : "muted"}
									hint={<ChangePill change={stats.data?.change_percent} />}
								/>
								<StatHero
									label={t('serverErrors', 'Server errors')}
									value={
										stats.isLoading
											? "…"
											: (stats.data?.server_errors ?? 0).toLocaleString()
									}
									icon={<ServerCrash className="h-4 w-4" />}
									tone={stats.data?.server_errors ? "destructive" : "muted"}
									hint={
										<span className="inline-flex items-center gap-1 text-destructive">
											<AlertTriangle className="h-3 w-3" />
											{t('5xxResponses', '5xx responses')}
										</span>
									}
								/>
								<StatHero
									label={t('clientErrors', 'Client errors')}
									value={
										stats.isLoading
											? "…"
											: (stats.data?.client_errors ?? 0).toLocaleString()
									}
									icon={<Zap className="h-4 w-4" />}
									tone={stats.data?.client_errors ? "warn" : "muted"}
									hint="4xx responses"
								/>
								<StatHero
									label={t('usersAffected', 'Users affected')}
									value={
										stats.isLoading
											? "…"
											: (
													stats.data?.unique_users_affected ?? 0
												).toLocaleString()
									}
									icon={<UsersRound className="h-4 w-4" />}
									hint={t('valPaths', '{{val}} paths', { val: stats.data?.unique_paths ?? 0 })}
								/>
							</div>

							<Card>
								<CardHeader className="pb-3">
									<CardTitle className="flex items-center gap-2 text-base">
										<Activity className="h-4 w-4" />
										{t('errorsOverTime', 'Errors over time')}
									</CardTitle>
									<CardDescription>
										{t('bucketedBy2', 'Bucketed by')}{" "}
										<span className="font-mono">
											{series.data?.bucket ?? "auto"}
										</span>{" "}
										{t('overTheSelectedWindow2', 'over the selected window.')}
									</CardDescription>
								</CardHeader>
								<CardContent>
									{series.isLoading ? (
										<Skeleton className="h-64 w-full" />
									) : (
										<ErrorChart
											points={series.data?.points ?? []}
											bucket={series.data?.bucket ?? "hour"}
										/>
									)}
								</CardContent>
							</Card>

							<div className="grid gap-4 lg:grid-cols-3">
								<DistributionCard
									title={t('topErrorCodes', 'Top error codes')}
									buckets={stats.data?.top_codes ?? []}
									loading={stats.isLoading}
									onPick={(b) => setFilterValue("public_code", b.key)}
									tone="destructive"
								/>
								<DistributionCard
									title={t('mostFailingPaths', 'Most failing paths')}
									buckets={stats.data?.top_paths ?? []}
									loading={stats.isLoading}
									onPick={(b) => setFilterValue("path", b.key)}
									tone="warn"
								/>
								<TopUsersCard
									buckets={stats.data?.top_users ?? []}
									loading={stats.isLoading}
									onPick={(b) => setFilterValue("user_id", b.key)}
								/>
							</div>

							<Card>
								<CardHeader className="space-y-3 pb-3">
									<div className="flex flex-wrap items-center justify-between gap-2">
										<CardTitle className="text-base">{t('errorLog', 'Error log')}</CardTitle>
										<CardDescription>
											{errorsQuery.data?.total ?? 0} {t('matchingReports', 'matching reports')}
										</CardDescription>
									</div>
									<div className="flex flex-wrap items-center gap-2">
										<div className="relative flex-1 min-w-[260px] max-w-md">
											<Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
											<Input
												placeholder={t('freetextSearchSummaryCodePathUser', 'Free-text search summary, code, path, user…')}
												value={filters.query}
												onChange={(e) =>
													setFilterValue("query", e.target.value)
												}
												className="pl-10"
											/>
										</div>
										<Input
											placeholder={`Error reference id (paste from user)`}
											value={filters.error_id}
											onChange={(e) =>
												setFilterValue("error_id", e.target.value)
											}
											className="w-72 font-mono text-xs"
										/>
										<Select
											value={filters.severity}
											onValueChange={(v) =>
												setFilterValue("severity", v as Severity)
											}
										>
											<SelectTrigger className="w-36">
												<SelectValue />
											</SelectTrigger>
											<SelectContent>
												<SelectItem value="all">{t('allSeverities', 'All severities')}</SelectItem>
												<SelectItem value="server">{t('server5xx', 'Server (5xx)')}</SelectItem>
												<SelectItem value="client">{t('client4xx', 'Client (4xx)')}</SelectItem>
											</SelectContent>
										</Select>
										<Select
											value={filters.method || "ALL"}
											onValueChange={(v) =>
												setFilterValue("method", v === "ALL" ? "" : v)
											}
										>
											<SelectTrigger className="w-32">
												<SelectValue placeholder="Method" />
											</SelectTrigger>
											<SelectContent>
												<SelectItem value="ALL">{t('allMethods', 'All methods')}</SelectItem>
												<SelectItem value="GET">GET</SelectItem>
												<SelectItem value="POST">{`POST`}</SelectItem>
												<SelectItem value="PUT">PUT</SelectItem>
												<SelectItem value="PATCH">{`PATCH`}</SelectItem>
												<SelectItem value="DELETE">{`DELETE`}</SelectItem>
											</SelectContent>
										</Select>
									</div>
									{activeFilterEntries.length > 0 && (
										<div className="flex flex-wrap items-center gap-2">
											<Filter className="h-3.5 w-3.5 text-muted-foreground" />
											{activeFilterEntries.map((f) => (
												<ActiveFilterChip
													key={f.key}
													label={f.label}
													value={f.value}
													onClear={() => {
														if (f.key === "severity") {
															setFilterValue("severity", "all");
														} else {
															setFilterValue(f.key, "" as never);
														}
													}}
												/>
											))}
											<Button
												size="sm"
												variant="ghost"
												onClick={handleClearAll}
												className="h-6 px-2 text-xs"
											>
												{t('clearAll', 'Clear all')}
											</Button>
										</div>
									)}
								</CardHeader>
								<CardContent className="p-0">
									{errorsQuery.isLoading ? (
										<div className="space-y-1.5 p-4">
											<Skeleton className="h-12 w-full" />
											<Skeleton className="h-12 w-full" />
											<Skeleton className="h-12 w-full" />
										</div>
									) : (errorsQuery.data?.errors.length ?? 0) === 0 ? (
										<div className="flex h-40 items-center justify-center text-sm text-muted-foreground">
											{t('noErrorsMatchTheCurrentFilters', 'No errors match the current filters.')}
										</div>
									) : (
										<ul className="divide-y divide-border">
											{errorsQuery.data?.errors.map((err) => {
												const tone = statusCodeTone(err.status_code);
												return (
													<li
														key={err.id}
														className="group cursor-pointer transition-colors hover:bg-muted/50"
													>
														<button
															type="button"
															onClick={() => openErrorDetail(err.id)}
															className="block w-full px-4 py-3 text-left"
														>
															<div className="flex items-center gap-2">
																<Badge
																	variant={tone.variant}
																	className="font-mono text-[11px]"
																>
																	{err.status_code}
																</Badge>
																<Badge
																	variant="outline"
																	className="font-mono text-[10px]"
																>
																	{err.method}
																</Badge>
																<code className="truncate font-mono text-[11px] text-muted-foreground">
																	{err.path}
																</code>
																<Badge
																	variant="secondary"
																	className="text-[10px]"
																>
																	{err.public_code}
																</Badge>
																<span className="ml-auto inline-flex items-center gap-2">
																	{err.user_id ? (
																		<UserPill userId={err.user_id} compact />
																	) : (
																		<Badge
																			variant="outline"
																			className="text-[10px]"
																		>
																			{t('anonymous', 'Anonymous')}
																		</Badge>
																	)}
																	<RelativeTime
																		value={err.created_at}
																		className="text-[11px] text-muted-foreground"
																	/>
																	<button
																		type="button"
																		onClick={(e) => {
																			e.stopPropagation();
																			navigator.clipboard
																				.writeText(err.id)
																				.catch(() => null);
																			toast.success("Reference id copied");
																		}}
																		className="opacity-0 transition-opacity group-hover:opacity-100"
																		title={t('copyReferenceId', 'Copy reference id')}
																	>
																		<Copy className="h-3.5 w-3.5 text-muted-foreground hover:text-foreground" />
																	</button>
																</span>
															</div>
															<div className="mt-1 line-clamp-2 text-sm">
																{err.summary}
															</div>
														</button>
													</li>
												);
											})}
										</ul>
									)}
								</CardContent>
							</Card>

							{totalPages > 1 && (
								<div className="flex items-center justify-between">
									<div className="text-sm text-muted-foreground">{t('pagePageOfTotalpages', 'Page {{page}} of {{totalPages}}', { page, totalPages })}</div>
									<div className="flex gap-2">
										<Button
											variant="outline"
											size="sm"
											onClick={() => setPage((p) => Math.max(1, p - 1))}
											disabled={page === 1}
										>
											{t('previous', 'Previous')}
										</Button>
										<Button
											variant="outline"
											size="sm"
											onClick={() =>
												setPage((p) => Math.min(totalPages, p + 1))
											}
											disabled={page >= totalPages}
										>
											{t('next', 'Next')}
										</Button>
									</div>
								</div>
							)}
						</TabsContent>

						<TabsContent value="audit">
							<AuditChainExplorer profile={profile.data} />
						</TabsContent>
					</Tabs>
				</div>
			</div>

			<ErrorDetailSheet
				errorId={selectedError}
				open={showDetail}
				onOpenChange={setShowDetail}
				profile={profile.data}
				onApplyFilter={handleApplyFilter}
			/>
		</main>
	);
}

function DistributionCard({
	title,
	buckets,
	loading,
	onPick,
	tone,
}: {
	title: string;
	buckets: { key: string; label: string; count: number }[];
	loading: boolean;
	onPick: (b: { key: string; label: string; count: number }) => void;
	tone: "destructive" | "warn";
}) {
	const { t } = useTranslation("admin");
	const max = Math.max(1, ...buckets.map((b) => b.count));
	const barColor =
		tone === "destructive" ? "bg-destructive/60" : "bg-amber-500/60";
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
					<div className="text-xs text-muted-foreground">{t('noData', 'No data')}</div>
				) : (
					<ul className="space-y-1.5">
						{buckets.map((b) => (
							<li key={b.key}>
								<button
									type="button"
									onClick={() => onPick(b)}
									className="group flex w-full items-center gap-2 rounded px-1 py-0.5 hover:bg-muted/60"
								>
									<span className="w-32 truncate text-xs font-medium group-hover:text-primary">
										{b.label}
									</span>
									<div className="relative flex-1 overflow-hidden rounded-full bg-muted h-2">
										<div
											className={`h-full rounded-full ${barColor}`}
											style={{ width: `${(b.count / max) * 100}%` }}
										/>
									</div>
									<span className="w-10 text-right text-[11px] tabular-nums text-muted-foreground">
										{b.count}
									</span>
								</button>
							</li>
						))}
					</ul>
				)}
			</CardContent>
		</Card>
	);
}

function TopUsersCard({
	buckets,
	loading,
	onPick,
}: {
	buckets: { key: string; label: string; count: number }[];
	loading: boolean;
	onPick: (b: { key: string; label: string; count: number }) => void;
}) {
	const { t } = useTranslation("admin");
	const max = Math.max(1, ...buckets.map((b) => b.count));
	return (
		<Card>
			<CardHeader className="pb-2">
				<CardTitle className="flex items-center gap-1.5 text-sm">
					<UsersRound className="h-3.5 w-3.5" /> {t('mostAffectedUsers', 'Most affected users')}
				</CardTitle>
			</CardHeader>
			<CardContent>
				{loading ? (
					<div className="space-y-1.5">
						<Skeleton className="h-6 w-full" />
						<Skeleton className="h-6 w-full" />
						<Skeleton className="h-6 w-full" />
					</div>
				) : buckets.length === 0 ? (
					<div className="text-xs text-muted-foreground">
						{t('noIdentifiedUsersInTheWindow', 'No identified users in the window.')}
					</div>
				) : (
					<ul className="space-y-1.5">
						{buckets.map((b) => (
							<li
								key={b.key}
								className="flex items-center gap-2 rounded px-1 py-0.5"
							>
								<UserPill userId={b.key} compact className="flex-1" />
								<div className="relative w-24 overflow-hidden rounded-full bg-muted h-2">
									<div
										className="h-full rounded-full bg-primary/70"
										style={{ width: `${(b.count / max) * 100}%` }}
									/>
								</div>
								<button
									type="button"
									onClick={() => onPick(b)}
									className="w-10 text-right text-[11px] tabular-nums text-muted-foreground hover:text-foreground"
									title={t('filterByThisUser', 'Filter by this user')}
								>
									{b.count}
								</button>
							</li>
						))}
					</ul>
				)}
			</CardContent>
		</Card>
	);
}

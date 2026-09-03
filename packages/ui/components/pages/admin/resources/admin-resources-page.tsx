"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	Database,
	HardDrive,
	Layers,
	type LucideIcon,
	RefreshCw,
	Server,
	TriangleAlert,
	Zap,
} from "lucide-react";
import { useMemo } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import { humanFileSize } from "../../../../lib/utils";
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
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "../../../ui";
import { BarList } from "../telemetry/telemetry-shared";
import { MetricValue, formatMetric } from "./resources-dashboard-widget";
import {
	type IAdminResourcesResponse,
	type IDatabaseDetail,
	type IMetricFreshness,
	type IMetricUnit,
	type IResourceKind,
	type IResourceMetric,
	type IResourceStatus,
	type ITableUsage,
	RESOURCES_ENDPOINT,
	healthTone,
	isFault,
} from "./types";

const KIND_ORDER: Record<IResourceKind, number> = {
	database: 0,
	cache: 1,
	stateStore: 2,
	storage: 3,
};

const KIND_ICON: Record<IResourceKind, LucideIcon> = {
	database: Database,
	cache: Zap,
	stateStore: Layers,
	storage: HardDrive,
};

const BLOAT_THRESHOLD = 0.2;

function metricOf(
	key: string,
	label: string,
	value: number,
	unit: IMetricUnit,
	freshness: IMetricFreshness = "live",
): IResourceMetric {
	return { key, label, value, unit, freshness };
}

/** The note is shown beneath the tile, so it must not also become a tooltip. */
function withoutNote(metric: IResourceMetric): IResourceMetric {
	if (!metric.note) return metric;
	return {
		key: metric.key,
		label: metric.label,
		value: metric.value,
		unit: metric.unit,
		freshness: metric.freshness,
		observedAt: metric.observedAt,
	};
}

function MetricTile({ metric }: { readonly metric: IResourceMetric }) {
	return (
		<div className="rounded-lg border border-border bg-muted/40 px-3 py-2">
			<div
				className="truncate text-[10px] uppercase tracking-wide text-muted-foreground"
				title={metric.label}
			>
				{metric.label}
			</div>
			<div className="mt-0.5">
				<MetricValue metric={withoutNote(metric)} />
			</div>
			{metric.note ? (
				<div className="mt-0.5 text-[11px] leading-snug text-muted-foreground">
					{metric.note}
				</div>
			) : null}
		</div>
	);
}

function FactTile({
	label,
	children,
}: {
	readonly label: string;
	readonly children: React.ReactNode;
}) {
	return (
		<div className="rounded-lg border border-border bg-muted/40 px-3 py-2">
			<div className="text-[10px] uppercase tracking-wide text-muted-foreground">
				{label}
			</div>
			<div className="mt-0.5 truncate text-sm font-medium">{children}</div>
		</div>
	);
}

function ResourceCard({ status }: { readonly status: IResourceStatus }) {
	const { t } = useTranslation("admin");
	const tone = healthTone(status.status);
	const fault = isFault(status.status);
	const KindIcon = KIND_ICON[status.kind];

	const healthLabel =
		status.status === "ok"
			? t("healthy", "Healthy")
			: status.status === "degraded"
				? t("degraded", "Degraded")
				: status.status === "unavailable"
					? t("unavailable", "Unavailable")
					: status.status === "unsupported"
						? t("noStatistics", "No statistics")
						: t("notConfigured", "Not configured");

	return (
		<Card className={fault ? "border-destructive/30" : undefined}>
			<CardHeader className="gap-2 pb-3">
				<div className="flex items-start justify-between gap-2">
					<CardTitle className="flex min-w-0 items-center gap-2 text-base">
						<KindIcon className="h-4 w-4 shrink-0 text-muted-foreground" />
						<span className="truncate" title={status.label}>
							{status.label}
						</span>
					</CardTitle>
					<span
						className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${tone.dot}`}
						aria-hidden="true"
					/>
				</div>
				<div className="flex flex-wrap items-center gap-1.5">
					<Badge variant="outline" className={`text-[10px] ${tone.badge}`}>
						{healthLabel}
					</Badge>
					<Badge variant="secondary" className="font-mono text-[10px]">
						{status.backend}
					</Badge>
					{typeof status.latencyMs === "number" ? (
						<span
							className="text-[11px] tabular-nums text-muted-foreground"
							title={t("probeLatency", "Probe latency")}
						>
							{formatMetric(
								metricOf(
									"latency_ms",
									t("probeLatency", "Probe latency"),
									status.latencyMs,
									"milliseconds",
								),
							)}
						</span>
					) : null}
				</div>
				{status.detail ? (
					<CardDescription
						className="truncate font-mono text-xs"
						title={status.detail}
					>
						{status.detail}
					</CardDescription>
				) : null}
			</CardHeader>
			<CardContent className="space-y-3">
				{status.metrics.length > 0 ? (
					<div className="grid grid-cols-2 gap-2">
						{status.metrics.map((metric) => (
							<MetricTile key={metric.key} metric={metric} />
						))}
					</div>
				) : null}
				{status.message ? (
					<p
						className={`text-xs leading-snug ${fault ? "text-destructive" : "text-muted-foreground"}`}
					>
						{status.message}
					</p>
				) : null}
				{status.metrics.length === 0 && !status.message ? (
					<p className="text-xs text-muted-foreground">
						{t(
							"noCheapStatisticsForThisResource",
							"No cheap statistics are available for this resource.",
						)}
					</p>
				) : null}
			</CardContent>
		</Card>
	);
}

function LargestTablesTable({
	tables,
}: {
	readonly tables: readonly ITableUsage[];
}) {
	const { t } = useTranslation("admin");

	return (
		<div className="overflow-x-auto">
			<Table>
				<TableHeader>
					<TableRow>
						<TableHead>{t("table", "Table")}</TableHead>
						<TableHead className="text-right">{t("total", "Total")}</TableHead>
						<TableHead className="text-right">
							{t("tableSize", "Table")}
						</TableHead>
						<TableHead className="text-right">
							{t("indexSize", "Indexes")}
						</TableHead>
						<TableHead className="text-right">
							<Tooltip>
								<TooltipTrigger asChild>
									<span className="cursor-help underline decoration-dotted underline-offset-2">
										{t("estimatedRows", "Est. rows")}
									</span>
								</TooltipTrigger>
								<TooltipContent>
									{t(
										"rowCountsComeFromThePlannerNotACount",
										"Row counts come from the planner statistics, not from COUNT(*).",
									)}
								</TooltipContent>
							</Tooltip>
						</TableHead>
						<TableHead className="text-right">
							{t("deadRows", "Dead rows")}
						</TableHead>
					</TableRow>
				</TableHeader>
				<TableBody>
					{tables.map((table) => {
						const share =
							table.estimatedRows > 0
								? table.deadRows / table.estimatedRows
								: 0;
						const bloated = table.estimatedRows > 0 && share > BLOAT_THRESHOLD;
						return (
							<TableRow key={table.name}>
								<TableCell className="font-mono text-xs">
									{table.name}
								</TableCell>
								<TableCell className="text-right text-xs tabular-nums">
									{humanFileSize(table.totalBytes)}
								</TableCell>
								<TableCell className="text-right text-xs tabular-nums text-muted-foreground">
									{humanFileSize(table.tableBytes)}
								</TableCell>
								<TableCell className="text-right text-xs tabular-nums text-muted-foreground">
									{humanFileSize(table.indexBytes)}
								</TableCell>
								<TableCell className="text-right text-xs tabular-nums text-muted-foreground">
									~{table.estimatedRows.toLocaleString()}
								</TableCell>
								<TableCell className="text-right text-xs tabular-nums">
									{bloated ? (
										<Tooltip>
											<TooltipTrigger asChild>
												<span className="inline-flex cursor-help items-center gap-1 text-amber-600 dark:text-amber-400">
													<TriangleAlert className="h-3 w-3" />~
													{table.deadRows.toLocaleString()}
												</span>
											</TooltipTrigger>
											<TooltipContent>
												{t(
													"deadRowShareIndicatesBloat",
													"{{percent}}% of the estimated rows are dead — bloat awaiting VACUUM.",
													{ percent: (share * 100).toFixed(0) },
												)}
											</TooltipContent>
										</Tooltip>
									) : (
										<span className="text-muted-foreground">
											~{table.deadRows.toLocaleString()}
										</span>
									)}
								</TableCell>
							</TableRow>
						);
					})}
				</TableBody>
			</Table>
		</div>
	);
}

function DatabaseSection({ detail }: { readonly detail: IDatabaseDetail }) {
	const { t } = useTranslation("admin");

	const connectionRows = useMemo(
		() =>
			detail.connections.map((connection) => ({
				key: connection.state,
				label: connection.state,
				count: connection.count,
			})),
		[detail.connections],
	);

	const rateMetrics = useMemo(() => {
		const rates = detail.rates;
		if (!rates) return [];
		return [
			metricOf(
				"commits",
				t("commits", "Commits"),
				rates.commits,
				"perSecond",
				"rate",
			),
			metricOf(
				"rollbacks",
				t("rollbacks", "Rollbacks"),
				rates.rollbacks,
				"perSecond",
				"rate",
			),
			metricOf(
				"tuples_read",
				t("tuplesRead", "Tuples read"),
				rates.tuplesRead,
				"perSecond",
				"rate",
			),
			metricOf(
				"tuples_written",
				t("tuplesWritten", "Tuples written"),
				rates.tuplesWritten,
				"perSecond",
				"rate",
			),
			metricOf(
				"blocks_read",
				t("blocksRead", "Blocks read"),
				rates.blocksRead,
				"perSecond",
				"rate",
			),
		];
	}, [detail.rates, t]);

	const counterMetrics = useMemo(() => {
		const counters = detail.counters;
		if (!counters) return [];
		return [
			metricOf("commits", t("commits", "Commits"), counters.commits, "count"),
			metricOf(
				"rollbacks",
				t("rollbacks", "Rollbacks"),
				counters.rollbacks,
				"count",
			),
			metricOf(
				"tuples_returned",
				t("tuplesReturned", "Tuples returned"),
				counters.tuplesReturned,
				"count",
			),
			metricOf(
				"tuples_fetched",
				t("tuplesFetched", "Tuples fetched"),
				counters.tuplesFetched,
				"count",
			),
			metricOf(
				"tuples_inserted",
				t("tuplesInserted", "Tuples inserted"),
				counters.tuplesInserted,
				"count",
			),
			metricOf(
				"tuples_updated",
				t("tuplesUpdated", "Tuples updated"),
				counters.tuplesUpdated,
				"count",
			),
			metricOf(
				"tuples_deleted",
				t("tuplesDeleted", "Tuples deleted"),
				counters.tuplesDeleted,
				"count",
			),
			metricOf(
				"blocks_hit",
				t("blocksHitCache", "Blocks hit (cache)"),
				counters.blocksHit,
				"count",
			),
			metricOf(
				"blocks_read",
				t("blocksReadDisk", "Blocks read (disk)"),
				counters.blocksRead,
				"count",
			),
			metricOf(
				"deadlocks",
				t("deadlocks", "Deadlocks"),
				counters.deadlocks,
				"count",
			),
			metricOf(
				"temp_files",
				t("tempFiles", "Temp files"),
				counters.tempFiles,
				"count",
			),
			metricOf(
				"temp_bytes",
				t("tempBytes", "Temp bytes"),
				counters.tempBytes,
				"bytes",
			),
		];
	}, [detail.counters, t]);

	return (
		<section className="space-y-4">
			<div>
				<h2 className="flex items-center gap-2 text-xl font-semibold">
					<Database className="h-5 w-5 text-muted-foreground" />
					{t("database", "Database")}
				</h2>
				<p className="text-sm text-muted-foreground">
					{t(
						"databaseSectionScope",
						"Statistics Postgres already keeps in memory — no table scans are issued to build this view.",
					)}
				</p>
			</div>

			<div className="grid gap-3 sm:grid-cols-3">
				<FactTile label={t("version", "Version")}>
					<span className="font-mono text-xs">{detail.version ?? "—"}</span>
				</FactTile>
				<FactTile label={t("databaseName", "Database")}>
					<span className="font-mono text-xs">
						{detail.databaseName ?? "—"}
					</span>
				</FactTile>
				<FactTile label={t("statisticsResetAt", "Statistics reset")}>
					{detail.statsResetAt ? (
						<RelativeTime value={detail.statsResetAt} className="text-sm" />
					) : (
						<span className="text-muted-foreground">
							{t("neverReset", "Never reset")}
						</span>
					)}
				</FactTile>
			</div>

			<div className="grid gap-4 lg:grid-cols-2">
				<Card>
					<CardHeader className="pb-3">
						<CardTitle className="text-base">
							{t("connectionsByState", "Connections by state")}
						</CardTitle>
						<CardDescription>
							{t(
								"backendsCurrentlyAttachedToThisDatabase",
								"Backends currently attached to this database.",
							)}
						</CardDescription>
					</CardHeader>
					<CardContent>
						<BarList
							rows={connectionRows}
							emptyMessage={t(
								"noConnectionsReported",
								"No connections reported.",
							)}
						/>
					</CardContent>
				</Card>

				<Card>
					<CardHeader className="pb-3">
						<CardTitle className="text-base">
							{t("throughput", "Throughput")}
						</CardTitle>
						<CardDescription>
							{detail.rates
								? t(
										"ratesDerivedFromTwoCounterSamples",
										"Rates derived from two counter samples.",
									)
								: t(
										"cumulativeCountersSinceTheLastStatisticsReset",
										"Cumulative counters since the last statistics reset.",
									)}
						</CardDescription>
					</CardHeader>
					<CardContent className="space-y-3">
						{detail.rates ? (
							<>
								<div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
									{rateMetrics.map((metric) => (
										<MetricTile key={metric.key} metric={metric} />
									))}
								</div>
								<p className="text-xs text-muted-foreground">
									{t(
										"measuredOverASecondsWindow",
										"Measured over a {{seconds}} s window between the last two samples.",
										{ seconds: detail.rates.windowSeconds },
									)}
								</p>
							</>
						) : counterMetrics.length > 0 ? (
							<>
								<p className="text-xs text-muted-foreground">
									{t(
										"ratesAppearOnceASecondSampleHasBeenTaken",
										"Per-second rates appear once a second sample has been taken. Until then the raw cumulative totals since the last statistics reset are shown.",
									)}
								</p>
								<dl className="grid gap-x-6 sm:grid-cols-2">
									{counterMetrics.map((metric) => (
										<div
											key={metric.key}
											className="flex items-baseline justify-between gap-2 border-b border-border/60 py-1"
										>
											<dt className="truncate text-xs text-muted-foreground">
												{metric.label}
											</dt>
											<dd className="text-xs font-medium tabular-nums">
												{formatMetric(metric)}
											</dd>
										</div>
									))}
								</dl>
							</>
						) : (
							<p className="text-xs text-muted-foreground">
								{t(
									"noThroughputCountersReported",
									"No throughput counters were reported by this database.",
								)}
							</p>
						)}
					</CardContent>
				</Card>
			</div>

			<Card>
				<CardHeader className="pb-3">
					<CardTitle className="text-base">
						{t("largestTables", "Largest tables")}
					</CardTitle>
					<CardDescription>
						{t(
							"sizesIncludeIndexesAndToastRowCountsAreEstimates",
							"Sizes include indexes and TOAST storage; row counts are planner estimates.",
						)}
					</CardDescription>
				</CardHeader>
				<CardContent className="pb-6">
					{detail.largestTables.length === 0 ? (
						<p className="text-xs text-muted-foreground">
							{t(
								"noTableStatisticsAvailable",
								"No table statistics available.",
							)}
						</p>
					) : (
						<LargestTablesTable tables={detail.largestTables} />
					)}
				</CardContent>
			</Card>
		</section>
	);
}

function ResourcesSkeleton() {
	return (
		<div className="space-y-6">
			<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
				<Skeleton className="h-56 w-full" />
				<Skeleton className="h-56 w-full" />
				<Skeleton className="h-56 w-full" />
				<Skeleton className="h-56 w-full" />
			</div>
			<Skeleton className="h-8 w-48" />
			<div className="grid gap-4 lg:grid-cols-2">
				<Skeleton className="h-56 w-full" />
				<Skeleton className="h-56 w-full" />
			</div>
			<Skeleton className="h-64 w-full" />
		</div>
	);
}

export function AdminResourcesPage() {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const resources = useQuery<IAdminResourcesResponse>({
		queryKey: ["admin", "resources"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<IAdminResourcesResponse>(
				profile.data,
				RESOURCES_ENDPOINT,
			);
		},
		enabled: !!profile.data,
		refetchInterval: 60_000,
	});

	const sorted = useMemo(
		() =>
			[...(resources.data?.resources ?? [])].sort((a, b) => {
				const byKind = KIND_ORDER[a.kind] - KIND_ORDER[b.kind];
				return byKind !== 0 ? byKind : a.id.localeCompare(b.id);
			}),
		[resources.data?.resources],
	);

	const loading = !resources.data && !resources.isError;

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-7xl space-y-6">
					<div className="flex flex-wrap items-start justify-between gap-3">
						<div className="space-y-1">
							<h1 className="flex items-center gap-2 text-3xl font-bold">
								<Server className="h-7 w-7 text-primary" />
								{t("resources", "Resources")}
							</h1>
							<p className="max-w-3xl text-muted-foreground">
								{t(
									"resourcesPageScope",
									"Every number here is either an O(1) probe or a rollup the provider already publishes — nothing on this page scans a bucket or a table. That is why storage sizes can lag by up to a day and why there is no exact object count.",
								)}
							</p>
							{resources.data ? (
								<div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
									<span>{t("collected", "Collected")}</span>
									<RelativeTime
										value={resources.data.generatedAt}
										className="text-xs text-muted-foreground"
									/>
									{resources.data.cached ? (
										<Badge variant="outline" className="text-[10px]">
											{t("cached", "cached")}
										</Badge>
									) : null}
								</div>
							) : null}
						</div>
						<Button
							variant="outline"
							size="sm"
							onClick={() => resources.refetch()}
							disabled={resources.isFetching}
						>
							<RefreshCw
								className={`mr-1 h-3.5 w-3.5 ${resources.isFetching ? "animate-spin" : ""}`}
							/>
							{t("refresh", "Refresh")}
						</Button>
					</div>

					{loading ? (
						<ResourcesSkeleton />
					) : resources.isError ? (
						<Card className="border-dashed">
							<CardHeader>
								<CardTitle className="text-base text-muted-foreground">
									{t(
										"resourceStatisticsUnavailable",
										"Resource statistics unavailable",
									)}
								</CardTitle>
								<CardDescription>
									{t(
										"theResourcesEndpointDidNotAnswer",
										"The admin/resources endpoint did not answer. The platform itself may still be healthy — only this read failed.",
									)}
								</CardDescription>
							</CardHeader>
							<CardContent>
								<p className="font-mono text-xs text-muted-foreground">
									{resources.error?.message ??
										t("unknownError", "Unknown error")}
								</p>
							</CardContent>
						</Card>
					) : (
						<>
							{sorted.length === 0 ? (
								<Card className="border-dashed">
									<CardHeader>
										<CardTitle className="text-base text-muted-foreground">
											{t("noResourcesReported", "No resources reported")}
										</CardTitle>
										<CardDescription>
											{t(
												"thisDeploymentDidNotReportAnyBackingServices",
												"This deployment did not report any backing services.",
											)}
										</CardDescription>
									</CardHeader>
								</Card>
							) : (
								<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
									{sorted.map((status) => (
										<ResourceCard key={status.id} status={status} />
									))}
								</div>
							)}

							{resources.data?.databaseDetail ? (
								<DatabaseSection detail={resources.data.databaseDetail} />
							) : null}
						</>
					)}
				</div>
			</div>
		</main>
	);
}

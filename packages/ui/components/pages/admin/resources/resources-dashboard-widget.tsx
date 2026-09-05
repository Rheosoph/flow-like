"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	ArrowRight,
	Boxes,
	Clock,
	Database,
	HardDrive,
	Info,
	Server,
	Zap,
} from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { cn, humanFileSize } from "../../../../lib/utils";
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
	TooltipTrigger,
} from "../../../ui";
import {
	type IAdminResourcesResponse,
	type IResourceHealth,
	type IResourceKind,
	type IResourceMetric,
	type IResourceStatus,
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

const KIND_ICON: Record<IResourceKind, typeof Database> = {
	database: Database,
	cache: Zap,
	stateStore: Boxes,
	storage: HardDrive,
};

/**
 * Value plus unit. Estimates keep their `~` here so the qualifier can never be
 * dropped by a call site that only wants the string.
 */
export function formatMetric(metric: IResourceMetric): string {
	if (!Number.isFinite(metric.value)) return "—";
	const prefix = metric.freshness === "estimate" ? "~" : "";
	switch (metric.unit) {
		case "bytes":
			return `${prefix}${humanFileSize(metric.value)}`;
		case "milliseconds":
			return `${prefix}${Math.round(metric.value).toLocaleString()} ms`;
		case "seconds":
			return `${prefix}${metric.value.toFixed(1)} s`;
		case "ratio":
			return `${prefix}${(metric.value * 100).toFixed(1)}%`;
		case "perSecond":
			return `${prefix}${
				Math.abs(metric.value) < 10
					? metric.value.toFixed(1)
					: Math.round(metric.value).toLocaleString()
			}/s`;
		default:
			return `${prefix}${Math.round(metric.value).toLocaleString()}`;
	}
}

/** First metric matching one of `keys`, in the order the keys are given. */
export function pickMetric(
	status: IResourceStatus,
	...keys: string[]
): IResourceMetric | undefined {
	for (const key of keys) {
		const match = status.metrics.find((metric) => metric.key === key);
		if (match) return match;
	}
	return undefined;
}

/**
 * The single place a metric is rendered. A provider rollup (S3 and GCS publish
 * storage size daily) always carries the instant it was measured, so nobody
 * reads a day-old bucket size as "now" and goes hunting for a deletion that
 * already happened.
 */
export function MetricValue({ metric }: Readonly<{ metric: IResourceMetric }>) {
	const { t } = useTranslation("admin");

	return (
		<span className="inline-flex items-baseline gap-1">
			<span className="text-sm font-semibold tabular-nums">
				{formatMetric(metric)}
			</span>
			{metric.freshness === "provider" ? (
				metric.observedAt ? (
					<RelativeTime
						value={metric.observedAt}
						style="narrow"
						className="text-[10px] text-muted-foreground"
					/>
				) : (
					<span className="text-[10px] text-muted-foreground">
						{t("providerMetric", "provider metric")}
					</span>
				)
			) : null}
			{metric.freshness === "estimate" ? (
				<span className="text-[10px] text-muted-foreground">
					{t("estimateShort", "est.")}
				</span>
			) : null}
			{metric.freshness === "rate" && metric.unit !== "perSecond" ? (
				<span className="text-[10px] text-muted-foreground">
					{t("rateShort", "rate")}
				</span>
			) : null}
			{metric.note ? (
				<Tooltip>
					<TooltipTrigger asChild>
						<Info className="h-3 w-3 shrink-0 text-muted-foreground" />
					</TooltipTrigger>
					<TooltipContent className="max-w-64 text-xs">
						{metric.note}
					</TooltipContent>
				</Tooltip>
			) : null}
		</span>
	);
}

function headlineFor(status: IResourceStatus): IResourceMetric | undefined {
	switch (status.kind) {
		case "database":
			return pickMetric(status, "size_bytes") ?? status.metrics[0];
		case "cache":
			return pickMetric(status, "entries", "size_bytes") ?? status.metrics[0];
		case "storage":
			return (
				pickMetric(status, "size_bytes", "disk_used_bytes", "object_count") ??
				status.metrics[0]
			);
		default:
			return status.latencyMs != null ? undefined : status.metrics[0];
	}
}

function ResourceRow({ status }: Readonly<{ status: IResourceStatus }>) {
	const { t } = useTranslation("admin");
	const tone = healthTone(status.status);
	const fault = isFault(status.status);
	const Icon = KIND_ICON[status.kind];
	const headline = headlineFor(status);
	const latencyIsHeadline = !headline && status.latencyMs != null;

	const statusWord: Record<IResourceHealth, string> = {
		ok: t("noStatistics", "No statistics"),
		degraded: t("degraded", "Degraded"),
		unavailable: t("unavailable", "Unavailable"),
		unsupported: t("unsupported", "Unsupported"),
		notConfigured: t("notConfigured", "Not configured"),
	};

	return (
		<li className="flex items-start gap-2.5 rounded-md border border-transparent px-2 py-1.5 transition-colors hover:border-border hover:bg-muted/40">
			<span
				className={cn("mt-1.5 h-2 w-2 shrink-0 rounded-full", tone.dot)}
				aria-hidden
			/>
			<Icon className="mt-0.5 h-3.5 w-3.5 shrink-0 text-muted-foreground" />
			<div className="min-w-0 flex-1">
				<div className="flex items-center gap-1.5">
					<span className="truncate text-xs font-medium">{status.label}</span>
					<Badge
						variant="outline"
						className="shrink-0 px-1.5 py-0 font-mono text-[10px] font-normal"
					>
						{status.backend}
					</Badge>
				</div>
				{status.detail ? (
					<div
						className="truncate text-[11px] text-muted-foreground"
						title={status.detail}
					>
						{status.detail}
					</div>
				) : null}
				{status.message ? (
					<div
						className={cn(
							"text-[11px] leading-snug",
							fault ? "text-destructive" : "text-muted-foreground",
						)}
					>
						{status.message}
					</div>
				) : null}
			</div>
			<div className="shrink-0 text-right">
				{headline ? (
					<MetricValue metric={headline} />
				) : latencyIsHeadline ? (
					<span className="text-sm font-semibold tabular-nums">
						{Math.round(status.latencyMs ?? 0).toLocaleString()} ms
					</span>
				) : (
					<span className="text-xs text-muted-foreground">
						{statusWord[status.status]}
					</span>
				)}
				{status.latencyMs != null && !latencyIsHeadline ? (
					<div className="text-[10px] tabular-nums text-muted-foreground">
						{Math.round(status.latencyMs).toLocaleString()} ms
					</div>
				) : null}
			</div>
		</li>
	);
}

export function DashboardResourcesWidget({
	profile,
}: Readonly<{ profile: IProfile | undefined }>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const query = useQuery<IAdminResourcesResponse>({
		queryKey: ["admin", "resources", profile?.hub, profile?.id],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<IAdminResourcesResponse>(
				profile,
				RESOURCES_ENDPOINT,
			);
		},
		enabled: !!profile,
		staleTime: 60_000,
		refetchInterval: 60_000,
		meta: { adminDashboard: true, persist: false },
	});

	const resources = useMemo(
		() =>
			[...(query.data?.resources ?? [])].sort(
				(a, b) =>
					KIND_ORDER[a.kind] - KIND_ORDER[b.kind] || a.id.localeCompare(b.id),
			),
		[query.data?.resources],
	);

	const faults = useMemo(
		() => resources.filter((resource) => isFault(resource.status)).length,
		[resources],
	);

	return (
		<Card className="overflow-hidden border-primary/20">
			<CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0 pb-3">
				<div className="space-y-1">
					<CardTitle className="flex items-center gap-2 text-base">
						<Server className="h-4 w-4 text-primary" />
						{t("resources", "Resources")}
					</CardTitle>
					<CardDescription>
						{query.data ? (
							<>
								{resources.length} {t("resourcesLower", "resources")}
								{faults > 0 ? (
									<>
										{" · "}
										<span className="text-destructive">
											{faults} {t("needsAttention", "needs attention")}
										</span>
									</>
								) : null}
							</>
						) : (
							t(
								"datastoresCachesAndBucketsBehindThisDeployment",
								"Datastores, caches and buckets behind this deployment",
							)
						)}
					</CardDescription>
				</div>
				<Button asChild size="sm" variant="ghost">
					<Link href="/admin/resources">
						{t("details", "Details")}
						<ArrowRight className="ml-1 h-3 w-3" />
					</Link>
				</Button>
			</CardHeader>
			<CardContent className="space-y-3">
				{query.isLoading ? (
					<div className="space-y-1.5">
						<Skeleton className="h-9 w-full" />
						<Skeleton className="h-9 w-full" />
						<Skeleton className="h-9 w-full" />
						<Skeleton className="h-9 w-full" />
					</div>
				) : query.isError ? (
					<div className="flex items-center justify-center rounded-lg border border-dashed py-6 text-sm text-muted-foreground">
						{t("couldNotReadResourceStatus", "Could not read resource status.")}
					</div>
				) : resources.length === 0 ? (
					<div className="flex items-center justify-center rounded-lg border border-dashed py-6 text-sm text-muted-foreground">
						{t("noResourcesReportedYet", "No resources reported yet.")}
					</div>
				) : (
					<ul className="-mx-2 space-y-0.5">
						{resources.map((resource) => (
							<ResourceRow key={resource.id} status={resource} />
						))}
					</ul>
				)}

				{query.data ? (
					<div className="flex flex-wrap items-center gap-x-2 gap-y-1 border-t pt-2 text-[11px] text-muted-foreground">
						<span className="inline-flex items-center gap-1">
							<Clock className="h-3 w-3" />
							{t("measured", "Measured")}
							<RelativeTime value={query.data.generatedAt} />
						</span>
						{query.data.cached ? (
							<span>
								{"· "}
								{t(
									"servedFromThe60SecondResponseCacheSoARefreshMayNotMoveTheNumbers",
									"served from the 60-second response cache, so a refresh may not move the numbers",
								)}
							</span>
						) : null}
					</div>
				) : null}
			</CardContent>
		</Card>
	);
}
